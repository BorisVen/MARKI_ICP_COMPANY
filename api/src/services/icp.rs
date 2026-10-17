//! Calls to the Marki payments canister signed by the custody key.
//!
//! Users without their own wallet keep their tokens in Marki custody: one ICRC-7
//! sub-account per Marki user under the custody principal. The server holds the
//! custody key (env `MARKI_CUSTODIAN_PEM`, an Ed25519 PEM) and moves tokens
//! between those sub-accounts when NFTs are gifted, or out to a user's own
//! wallet on request. Without the key, custody is simply off.

use candid::{CandidType, Deserialize, Nat, Principal};
use ic_agent::{identity::BasicIdentity, Agent};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

const DEFAULT_CANISTER: &str = "dbrm3-yqaaa-aaaak-qzjeq-cai";
const DEFAULT_HOST: &str = "https://icp-api.io";

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Account {
    pub owner: Principal,
    pub subaccount: Option<serde_bytes::ByteBuf>,
}

#[derive(CandidType)]
struct TransferArg {
    from_subaccount: Option<serde_bytes::ByteBuf>,
    to: Account,
    token_id: Nat,
    memo: Option<serde_bytes::ByteBuf>,
    created_at_time: Option<u64>,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum TransferError {
    NonExistingTokenId,
    InvalidRecipient,
    Unauthorized,
    GenericError { error_code: Nat, message: String },
}

pub struct Custody {
    agent: Agent,
    canister: Principal,
    pub custodian: Principal,
}

static CUSTODY: OnceCell<Option<Custody>> = OnceCell::const_new();

/// The custody connection, or `None` when no key is configured.
pub async fn custody() -> Option<&'static Custody> {
    CUSTODY
        .get_or_init(|| async {
            let pem = std::env::var("MARKI_CUSTODIAN_PEM").ok().filter(|p| !p.trim().is_empty())?;
            let identity = match BasicIdentity::from_pem(pem.replace("\\n", "\n").as_bytes()) {
                Ok(i) => i,
                Err(e) => {
                    tracing::error!(error = %e, "MARKI_CUSTODIAN_PEM is not a valid Ed25519 PEM; custody is off");
                    return None;
                }
            };
            let host = std::env::var("ICP_HOST").unwrap_or_else(|_| DEFAULT_HOST.into());
            let agent = Agent::builder().with_url(host).with_identity(identity).build().ok()?;
            let custodian = agent.get_principal().ok()?;
            let canister = std::env::var("ICP_PAYMENTS_CANISTER").unwrap_or_else(|_| DEFAULT_CANISTER.into());
            let canister = Principal::from_text(canister).ok()?;
            tracing::info!(%custodian, %canister, "Marki custody is on");
            Some(Custody { agent, canister, custodian })
        })
        .await
        .as_ref()
}

/// The custody sub-account of a Marki user: sha256("marki-custody:" + uid).
pub fn user_subaccount(uid: &str) -> [u8; 32] {
    Sha256::digest(format!("marki-custody:{uid}").as_bytes()).into()
}

/// `icp:<canister>:<token>` → token id, when it is a token of `canister`.
pub fn token_of(mint_address: &str, canister: &Principal) -> Option<u64> {
    let mut parts = mint_address.split(':');
    (parts.next()? == "icp" && parts.next()? == canister.to_text()).then_some(())?;
    parts.next()?.parse().ok()
}

impl Custody {
    pub fn canister(&self) -> Principal {
        self.canister
    }

    pub fn account_of(&self, uid: &str) -> Account {
        Account { owner: self.custodian, subaccount: Some(serde_bytes::ByteBuf::from(user_subaccount(uid).to_vec())) }
    }

    /// Activates a card-paid plan for `owner` (the canister lets only this key do it).
    pub async fn grant_plan(&self, owner: Principal, plan_id: &str) -> anyhow::Result<Result<candid::Reserved, String>> {
        let bytes = self
            .agent
            .update(&self.canister, "grant_plan_by_operator")
            .with_arg(candid::encode_args((owner, plan_id.to_owned()))?)
            .await?;
        Ok(candid::decode_one::<Result<candid::Reserved, String>>(&bytes)?)
    }

    /// Moves tokens out of `from_uid`'s custody sub-account to `to`.
    /// Returns one result per token (Unauthorized: not in that sub-account).
    pub async fn transfer(&self, from_uid: &str, to: &Account, token_ids: &[u64]) -> anyhow::Result<Vec<Result<Nat, TransferError>>> {
        let from = serde_bytes::ByteBuf::from(user_subaccount(from_uid).to_vec());
        let args: Vec<TransferArg> = token_ids
            .iter()
            .map(|id| TransferArg {
                from_subaccount: Some(from.clone()),
                to: to.clone(),
                token_id: Nat::from(*id),
                memo: None,
                created_at_time: None,
            })
            .collect();
        let mut out = Vec::with_capacity(args.len());
        for chunk in args.chunks(100) {
            let bytes = self
                .agent
                .update(&self.canister, "icrc7_transfer")
                .with_arg(candid::encode_one(chunk)?)
                .await?;
            let res: Vec<Option<Result<Nat, TransferError>>> = candid::decode_one(&bytes)?;
            out.extend(res.into_iter().map(|r| r.unwrap_or(Err(TransferError::GenericError {
                error_code: Nat::from(0u8),
                message: "no result".into(),
            }))));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subaccounts_are_stable_and_distinct() {
        assert_eq!(user_subaccount("a"), user_subaccount("a"));
        assert_ne!(user_subaccount("a"), user_subaccount("b"));
        // The same hash the web app computes: sha256("marki-custody:uid").
        assert_eq!(hex::encode(user_subaccount("uid")), hex::encode(Sha256::digest(b"marki-custody:uid")));
    }

    #[test]
    fn mint_addresses_parse_for_our_canister_only() {
        let c = Principal::from_text(DEFAULT_CANISTER).unwrap();
        assert_eq!(token_of("icp:dbrm3-yqaaa-aaaak-qzjeq-cai:206", &c), Some(206));
        assert_eq!(token_of("icp:aaaaa-aa:206", &c), None);
        assert_eq!(token_of("solana:xyz", &c), None);
    }
}

#[cfg(test)]
mod live {
    /// Signs a real call to the mainnet canister with the custody key:
    /// `MARKI_CUSTODIAN_PEM="$(cat key.pem)" cargo test live -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn custody_key_reaches_the_canister() {
        let c = super::custody().await.expect("custody key configured");
        let to = c.account_of("live-test-receiver");
        // Token 0 isn't in this sub-account: the canister must refuse.
        let res = c.transfer("live-test-sender", &to, &[0]).await.unwrap();
        assert!(matches!(res[0], Err(super::TransferError::Unauthorized)), "{res:?}");
    }
}
