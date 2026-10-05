use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::Config;
use super::google_auth::GoogleAuthService;

// ── Firestore client ──────────────────────────────────────────────────────────

pub struct FirestoreClient {
    http: reqwest::Client,
    project_id: String,
    auth: Arc<GoogleAuthService>,
}

impl FirestoreClient {
    pub fn new(config: Arc<Config>, auth: Arc<GoogleAuthService>) -> Self {
        FirestoreClient {
            http: reqwest::Client::new(),
            project_id: config.firebase_project_id.clone(),
            auth,
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn base_url(&self) -> String {
        format!(
            "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents",
            self.project_id
        )
    }

    async fn token(&self) -> Result<String> {
        self.auth.access_token().await
    }

    // ── CRUD operations ───────────────────────────────────────────────────────

    /// Fetch a single document.  Returns `None` if the document does not exist.
    pub async fn get(&self, collection: &str, doc_id: &str) -> Result<Option<Value>> {
        let token = self.token().await?;
        let url = format!("{}/{}/{}", self.base_url(), collection, doc_id);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .context("Firestore GET request failed")?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Firestore get error ({}): {}", collection, body));
        }

        let doc: Value = resp.json().await.context("Failed to parse Firestore document")?;
        Ok(Some(doc_to_json(&doc)))
    }

    /// Write (create or overwrite) a document at a known ID.
    pub async fn set(&self, collection: &str, doc_id: &str, data: &Value) -> Result<()> {
        let token = self.token().await?;
        let url = format!("{}/{}/{}", self.base_url(), collection, doc_id);

        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&token)
            .json(&json_to_doc(data))
            .send()
            .await
            .context("Firestore SET request failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Firestore set error ({}): {}", collection, body));
        }

        Ok(())
    }

    /// Create a document with a Firestore-generated ID.  Returns the new ID.
    pub async fn create(&self, collection: &str, data: &Value) -> Result<String> {
        let token = self.token().await?;
        let url = format!("{}/{}", self.base_url(), collection);

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&json_to_doc(data))
            .send()
            .await
            .context("Firestore CREATE request failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Firestore create error ({}): {}", collection, body));
        }

        let doc: Value = resp.json().await.context("Failed to parse created document")?;
        let name = doc["name"].as_str().unwrap_or_default();
        let id = name.split('/').last().unwrap_or_default().to_owned();

        Ok(id)
    }

    /// Update only the specified fields in an existing document (field mask patch).
    pub async fn update(&self, collection: &str, doc_id: &str, fields: &Value) -> Result<()> {
        let token = self.token().await?;

        // Build `?updateMask.fieldPaths=f1&updateMask.fieldPaths=f2` manually to
        // ensure repeated query params are rendered correctly.
        let field_names: Vec<String> = fields
            .as_object()
            .map(|obj| obj.keys().filter(|k| k.as_str() != "id").cloned().collect())
            .unwrap_or_default();

        if field_names.is_empty() {
            return Ok(());
        }

        let mask_qs: String = field_names
            .iter()
            .map(|f| format!("updateMask.fieldPaths={}", urlencoding::encode(f)))
            .collect::<Vec<_>>()
            .join("&");

        let url = format!(
            "{}/{}/{}?{}",
            self.base_url(),
            collection,
            doc_id,
            mask_qs
        );

        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&token)
            .json(&json_to_doc(fields))
            .send()
            .await
            .context("Firestore UPDATE request failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Firestore update error ({}): {}", collection, body));
        }

        Ok(())
    }

    /// Delete a document.  Succeeds even if the document does not exist.
    pub async fn delete(&self, collection: &str, doc_id: &str) -> Result<()> {
        let token = self.token().await?;
        let url = format!("{}/{}/{}", self.base_url(), collection, doc_id);

        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .context("Firestore DELETE request failed")?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Firestore delete error ({}): {}", collection, body));
        }

        Ok(())
    }

    /// Run a structured query against a collection.
    ///
    /// `filters` — zero or more equality / array-contains conditions (AND-ed).
    /// `order_by` — optional `(field_path, descending)`.
    /// `limit`    — optional maximum number of results.
    pub async fn query(
        &self,
        collection: &str,
        filters: Vec<QueryFilter>,
        order_by: Option<(&str, bool)>,
        limit: Option<i32>,
    ) -> Result<Vec<Value>> {
        let token = self.token().await?;
        // runQuery must be called on the database root, not a collection path.
        let url = format!(
            "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents:runQuery",
            self.project_id
        );

        // ── Build structured query body ───────────────────────────────────────
        let from = json!([{ "collectionId": collection }]);

        let where_clause: Option<Value> = match filters.len() {
            0 => None,
            1 => {
                let f = &filters[0];
                Some(build_field_filter(&f.field, &f.op, &f.value))
            }
            _ => {
                let conditions: Vec<Value> = filters
                    .iter()
                    .map(|f| build_field_filter(&f.field, &f.op, &f.value))
                    .collect();
                Some(json!({
                    "compositeFilter": {
                        "op": "AND",
                        "filters": conditions
                    }
                }))
            }
        };

        let order_by_clause: Option<Value> = order_by.map(|(field, desc)| {
            json!([{
                "field": { "fieldPath": field },
                "direction": if desc { "DESCENDING" } else { "ASCENDING" }
            }])
        });

        let mut structured_query = serde_json::Map::new();
        structured_query.insert("from".to_owned(), from);
        if let Some(w) = where_clause {
            structured_query.insert("where".to_owned(), w);
        }
        if let Some(o) = order_by_clause {
            structured_query.insert("orderBy".to_owned(), o);
        }
        if let Some(l) = limit {
            structured_query.insert("limit".to_owned(), json!(l));
        }

        let body = json!({ "structuredQuery": Value::Object(structured_query) });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .context("Firestore QUERY request failed")?;

        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Firestore query error ({}): {}",
                collection,
                body_text
            ));
        }

        // runQuery returns a JSON array; each element may contain a "document" key
        // (missing if the result is a "no match" sentinel).
        let results: Vec<Value> = resp.json().await.context("Failed to parse query results")?;

        let docs: Vec<Value> = results
            .into_iter()
            .filter_map(|r| r.get("document").cloned())
            .map(|doc| doc_to_json(&doc))
            .collect();

        Ok(docs)
    }

    /// List all documents in a sub-collection path, e.g.
    /// `marki_wallets/{uid}/nfts`.
    pub async fn list_subcollection(
        &self,
        collection: &str,
        doc_id: &str,
        subcollection: &str,
    ) -> Result<Vec<Value>> {
        let token = self.token().await?;
        let url = format!(
            "{}/{}/{}/{}",
            self.base_url(),
            collection,
            doc_id,
            subcollection
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .context("Firestore list-subcollection request failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Firestore list error ({}/{}/{}): {}",
                collection,
                doc_id,
                subcollection,
                body
            ));
        }

        let result: Value = resp.json().await.context("Failed to parse subcollection list")?;

        let docs: Vec<Value> = result["documents"]
            .as_array()
            .map(|arr| arr.iter().map(|d| doc_to_json(d)).collect())
            .unwrap_or_default();

        Ok(docs)
    }

    /// Convenience: set a document inside a sub-collection path, e.g.
    /// `marki_wallets/{uid}/nfts/{nft_id}`.
    pub async fn set_subcollection(
        &self,
        collection: &str,
        doc_id: &str,
        subcollection: &str,
        sub_doc_id: &str,
        data: &Value,
    ) -> Result<()> {
        let path = format!("{}/{}/{}", collection, doc_id, subcollection);
        self.set(&path, sub_doc_id, data).await
    }

    /// Convenience: create a document inside a sub-collection path.
    pub async fn create_subcollection(
        &self,
        collection: &str,
        doc_id: &str,
        subcollection: &str,
        data: &Value,
    ) -> Result<String> {
        let path = format!("{}/{}/{}", collection, doc_id, subcollection);
        self.create(&path, data).await
    }

    /// Convenience: delete a document inside a sub-collection path.
    pub async fn delete_subcollection(
        &self,
        collection: &str,
        doc_id: &str,
        subcollection: &str,
        sub_doc_id: &str,
    ) -> Result<()> {
        let path = format!("{}/{}/{}", collection, doc_id, subcollection);
        self.delete(&path, sub_doc_id).await
    }
}

// ── Query filter builder ──────────────────────────────────────────────────────

/// A single filter condition for `FirestoreClient::query`.
pub struct QueryFilter {
    pub field: String,
    pub op: String,
    pub value: Value,
}

impl QueryFilter {
    pub fn equal(field: impl Into<String>, value: impl Into<Value>) -> Self {
        QueryFilter { field: field.into(), op: "EQUAL".to_owned(), value: value.into() }
    }

    pub fn array_contains(field: impl Into<String>, value: impl Into<Value>) -> Self {
        QueryFilter { field: field.into(), op: "ARRAY_CONTAINS".to_owned(), value: value.into() }
    }

    pub fn not_equal(field: impl Into<String>, value: impl Into<Value>) -> Self {
        QueryFilter { field: field.into(), op: "NOT_EQUAL".to_owned(), value: value.into() }
    }
}

fn build_field_filter(field: &str, op: &str, value: &Value) -> Value {
    json!({
        "fieldFilter": {
            "field": { "fieldPath": field },
            "op": op,
            "value": to_fs(value)
        }
    })
}

// ── Firestore wire-format conversion ─────────────────────────────────────────
//
// Firestore REST uses typed wrapper objects for every value, e.g.
//   `{ "stringValue": "hello" }`  or  `{ "integerValue": "42" }`
//
// The helpers below convert between that wire format and plain serde_json::Value.

/// Convert a plain JSON value → Firestore REST typed value.
pub fn to_fs(v: &Value) -> Value {
    match v {
        Value::String(s) => json!({ "stringValue": s }),
        Value::Bool(b) => json!({ "booleanValue": b }),
        Value::Number(n) => {
            if n.is_f64() {
                json!({ "doubleValue": n })
            } else {
                // Firestore represents integers as strings in the REST API.
                json!({ "integerValue": n.to_string() })
            }
        }
        Value::Null => json!({ "nullValue": null }),
        Value::Array(arr) => {
            let values: Vec<Value> = arr.iter().map(to_fs).collect();
            json!({ "arrayValue": { "values": values } })
        }
        Value::Object(obj) => {
            let fields: serde_json::Map<String, Value> =
                obj.iter().map(|(k, v)| (k.clone(), to_fs(v))).collect();
            json!({ "mapValue": { "fields": fields } })
        }
    }
}

/// Convert a Firestore REST typed value → plain JSON value.
pub fn from_fs(v: &Value) -> Value {
    if let Some(s) = v.get("stringValue") {
        return s.clone();
    }
    if let Some(b) = v.get("booleanValue") {
        return b.clone();
    }
    if let Some(n) = v.get("integerValue") {
        // Firestore sends integer values as JSON strings.
        if let Some(s) = n.as_str() {
            if let Ok(i) = s.parse::<i64>() {
                return json!(i);
            }
        }
        return n.clone();
    }
    if let Some(n) = v.get("doubleValue") {
        return n.clone();
    }
    if v.get("nullValue").is_some() {
        return Value::Null;
    }
    if let Some(arr_node) = v.get("arrayValue") {
        let items = arr_node
            .get("values")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(from_fs).collect::<Vec<_>>())
            .unwrap_or_default();
        return json!(items);
    }
    if let Some(map_node) = v.get("mapValue") {
        if let Some(fields) = map_node.get("fields").and_then(Value::as_object) {
            let converted: serde_json::Map<String, Value> =
                fields.iter().map(|(k, v)| (k.clone(), from_fs(v))).collect();
            return Value::Object(converted);
        }
        return json!({});
    }
    // Unknown type — return null rather than panicking.
    Value::Null
}

/// Convert a full Firestore document object (with "name" + "fields") to a plain
/// JSON object.  The document's ID is injected as an `"id"` field.
fn doc_to_json(doc: &Value) -> Value {
    let mut map = serde_json::Map::new();

    // Extract document ID from the resource name, e.g.
    // `projects/p/databases/(default)/documents/users/uid123`
    if let Some(name) = doc.get("name").and_then(Value::as_str) {
        if let Some(id) = name.split('/').last() {
            map.insert("id".to_owned(), json!(id));
        }
    }

    if let Some(fields) = doc.get("fields").and_then(Value::as_object) {
        for (key, typed_value) in fields {
            map.insert(key.clone(), from_fs(typed_value));
        }
    }

    Value::Object(map)
}

/// Convert a plain JSON object to a Firestore document body `{ "fields": { … } }`.
/// The `"id"` field is stripped because Firestore stores the ID in the resource
/// name, not in the document fields.
fn json_to_doc(data: &Value) -> Value {
    let fields: serde_json::Map<String, Value> = data
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| k.as_str() != "id")
                .map(|(k, v)| (k.clone(), to_fs(v)))
                .collect()
        })
        .unwrap_or_default();

    json!({ "fields": fields })
}
