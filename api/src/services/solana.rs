use anyhow::{Context, Result};
use serde_json::json;

/// Provides Solana-specific operations: address validation (pure, no network)
/// and balance fetching (via the configured JSON-RPC endpoint).
pub struct SolanaService {
    rpc_url: String,
    http: reqwest::Client,
}

impl SolanaService {
    pub fn new(rpc_url: String) -> Self {
        SolanaService {
            rpc_url,
            http: reqwest::Client::new(),
        }
    }

    // ── Address validation ────────────────────────────────────────────────────

    /// Return `true` if `address` is a valid Solana public key.
    ///
    /// A valid Solana address is a base58-encoded 32-byte Ed25519 public key.
    /// This check is pure (no network call) and runs in the `POST /api/wallets/crypto`
    /// handler before any Firestore write.
    pub fn is_valid_address(&self, address: &str) -> bool {
        match bs58::decode(address).into_vec() {
            Ok(bytes) => bytes.len() == 32,
            Err(_) => false,
        }
    }

    // ── Balance fetching ──────────────────────────────────────────────────────

    /// Fetch the confirmed SOL balance for `address` via the Solana JSON-RPC.
    ///
    /// Returns the balance in **SOL** (not lamports).
    /// `1 SOL = 1_000_000_000 lamports`.
    pub async fn get_balance(&self, address: &str) -> Result<f64> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBalance",
            "params": [
                address,
                { "commitment": "confirmed" }
            ]
        });

        let resp = self
            .http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("Solana RPC request failed")?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Solana RPC HTTP error: {}", err_body));
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Solana RPC response")?;

        // The RPC may return an application-level error even with HTTP 200.
        if let Some(err) = result.get("error") {
            return Err(anyhow::anyhow!("Solana RPC error: {}", err));
        }

        let lamports = result["result"]["value"]
            .as_u64()
            .context("Missing or invalid 'value' in Solana getBalance response")?;

        Ok(lamports as f64 / 1_000_000_000.0)
    }

    /// Fetch balances for multiple addresses concurrently using
    /// `getMultipleAccounts` — more efficient than individual calls when
    /// refreshing several wallets at once.
    ///
    /// Returns a `Vec` of `(address, balance_sol)` in the same order as the
    /// input slice.  Addresses that fail to resolve are returned with `0.0`.
    pub async fn get_balances_batch(&self, addresses: &[&str]) -> Vec<(String, f64)> {
        // Fire all requests concurrently with tokio::join-style collect.
        let futures: Vec<_> = addresses
            .iter()
            .map(|addr| {
                let addr = addr.to_string();
                async move {
                    let balance = self.get_balance(&addr).await.unwrap_or(0.0);
                    (addr, balance)
                }
            })
            .collect();

        futures::future::join_all(futures).await
    }

    /// Convert a listing amount to SOL using Coinbase's unauthenticated spot
    /// price endpoint.
    /// SOL listings need no network lookup. Fiat quotes are intentionally
    /// created on the server so the client cannot choose the amount paid.
    pub async fn listing_price_in_sol(&self, amount: f64, currency: &str) -> Result<f64> {
        if !amount.is_finite() || amount <= 0.0 {
            return Err(anyhow::anyhow!("Listing price must be greater than zero"));
        }

        let normalized = currency.trim().to_ascii_uppercase();
        if normalized == "SOL" {
            return Ok(amount);
        }
        if normalized != "UAH" && normalized != "USD" {
            return Err(anyhow::anyhow!(
                "Phantom payment is not supported for {normalized} listings"
            ));
        }

        let pair = format!("SOL-{normalized}");
        let response = self
            .http
            .get(format!("https://api.coinbase.com/v2/prices/{pair}/spot"))
            .header(reqwest::header::USER_AGENT, "MarkIdentity/2.0")
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .context("Failed to request the SOL exchange rate")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "SOL exchange-rate service returned {}",
                response.status()
            ));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse the SOL exchange rate")?;
        let fiat_per_sol = body["data"]["amount"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .context("SOL exchange rate is unavailable")?;

        Ok(amount / fiat_per_sol)
    }

    /// Verify that a confirmed transaction paid the exact listing price to the
    /// seller and the exact platform fee to the treasury.
    pub async fn verify_payment(
        &self,
        signature: &str,
        payer: &str,
        seller: &str,
        seller_lamports: u64,
        treasury: &str,
        fee_lamports: u64,
    ) -> Result<()> {
        if bs58::decode(signature).into_vec().map(|v| v.len()).unwrap_or(0) != 64 {
            return Err(anyhow::anyhow!("Invalid Solana transaction signature"));
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [
                signature,
                {
                    "commitment": "confirmed",
                    "encoding": "jsonParsed",
                    "maxSupportedTransactionVersion": 0
                }
            ]
        });

        let response = self
            .http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("Solana payment verification request failed")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Solana payment verification HTTP error: {}",
                response.status()
            ));
        }

        let rpc: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Solana payment verification response")?;
        if let Some(error) = rpc.get("error") {
            return Err(anyhow::anyhow!("Solana RPC error: {}", error));
        }

        validate_payment_transaction(
            &rpc["result"],
            payer,
            seller,
            seller_lamports,
            treasury,
            fee_lamports,
        )
    }
}

fn validate_payment_transaction(
    transaction: &serde_json::Value,
    payer: &str,
    seller: &str,
    seller_lamports: u64,
    treasury: &str,
    fee_lamports: u64,
) -> Result<()> {
    if transaction.is_null() {
        return Err(anyhow::anyhow!("Payment transaction was not found or is not confirmed"));
    }
    if !transaction["meta"]["err"].is_null() {
        return Err(anyhow::anyhow!("Payment transaction failed on-chain"));
    }

    let payer_signed = transaction["transaction"]["message"]["accountKeys"]
        .as_array()
        .map(|keys| {
            keys.iter().any(|key| {
                key["pubkey"].as_str() == Some(payer)
                    && key["signer"].as_bool() == Some(true)
            })
        })
        .unwrap_or(false);
    if !payer_signed {
        return Err(anyhow::anyhow!("Payment was not signed by the connected wallet"));
    }

    let instructions = transaction["transaction"]["message"]["instructions"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Payment transaction has no instructions"))?;

    let has_transfer = |destination: &str, lamports: u64| {
        instructions.iter().any(|instruction| {
            instruction["program"].as_str() == Some("system")
                && instruction["parsed"]["type"].as_str() == Some("transfer")
                && instruction["parsed"]["info"]["source"].as_str() == Some(payer)
                && instruction["parsed"]["info"]["destination"].as_str() == Some(destination)
                && instruction["parsed"]["info"]["lamports"].as_u64() == Some(lamports)
        })
    };

    if !has_transfer(seller, seller_lamports) {
        return Err(anyhow::anyhow!("Seller payment does not match the listing price"));
    }
    if fee_lamports > 0 && !has_transfer(treasury, fee_lamports) {
        return Err(anyhow::anyhow!("Platform fee payment is missing or incorrect"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_payment_transaction;
    use serde_json::json;

    #[test]
    fn accepts_exact_signed_payment() {
        let transaction = json!({
            "meta": { "err": null },
            "transaction": {
                "message": {
                    "accountKeys": [
                        { "pubkey": "payer", "signer": true },
                        { "pubkey": "seller", "signer": false },
                        { "pubkey": "treasury", "signer": false }
                    ],
                    "instructions": [
                        {
                            "program": "system",
                            "parsed": {
                                "type": "transfer",
                                "info": {
                                    "source": "payer",
                                    "destination": "seller",
                                    "lamports": 100
                                }
                            }
                        },
                        {
                            "program": "system",
                            "parsed": {
                                "type": "transfer",
                                "info": {
                                    "source": "payer",
                                    "destination": "treasury",
                                    "lamports": 1
                                }
                            }
                        }
                    ]
                }
            }
        });

        assert!(validate_payment_transaction(
            &transaction, "payer", "seller", 100, "treasury", 1
        ).is_ok());
    }

    #[test]
    fn rejects_wrong_seller_amount() {
        let transaction = json!({
            "meta": { "err": null },
            "transaction": {
                "message": {
                    "accountKeys": [{ "pubkey": "payer", "signer": true }],
                    "instructions": [{
                        "program": "system",
                        "parsed": {
                            "type": "transfer",
                            "info": {
                                "source": "payer",
                                "destination": "seller",
                                "lamports": 99
                            }
                        }
                    }]
                }
            }
        });

        assert!(validate_payment_transaction(
            &transaction, "payer", "seller", 100, "treasury", 1
        ).is_err());
    }
}
