// This file is part of Encrypted mempool.

use encryptor::SequencerPoseidonEncryption;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ObjectParams;
use jsonrpsee::rpc_params;
use jsonrpsee::ws_client::WsClientBuilder;
use mp_starknet::transaction::types::{EncryptedInvokeTransaction, InvokeTransaction};
use serde_json::json;
use vdf::VDF;
/// Decryptor has delay function for calculate decryption key and
/// decrypt function for decryption with poseidon algorithm
pub struct Decryptor {
    decrypt_function: SequencerPoseidonEncryption,
    delay_function: VDF,
}

impl Default for Decryptor {
    fn default() -> Self {
        let base = 10; // Expression base (e.g. 10 == decimal / 16 == hex)
        let lambda = 2048; // N's bits (ex. RSA-2048 => lambda = 2048)

        Self { decrypt_function: SequencerPoseidonEncryption::new(), delay_function: VDF::new(lambda, base) }
    }
}

impl Decryptor {
    /// Generate a decryptor.
    pub fn new() -> Self {
        Decryptor::default()
    }

    /// Decrypt encrypted invoke transaction
    pub async fn decrypt_encrypted_invoke_transaction(
        &self,
        encrypted_invoke_transaction: EncryptedInvokeTransaction,
        decryption_key: Option<String>,
    ) -> InvokeTransaction {
        println!("Decrypting encrypted invoke transaction... using internal decryptor");
        let symmetric_key = decryption_key.unwrap_or_else(|| {
            // 2. Use naive
            self.delay_function.evaluate(
                encrypted_invoke_transaction.t,
                encrypted_invoke_transaction.g.clone(),
                encrypted_invoke_transaction.n.clone(),
            )
        });

        let symmetric_key = SequencerPoseidonEncryption::calculate_secret_key(symmetric_key.as_bytes());

        let decrypted_invoke_tx = self.decrypt_function.decrypt(
            encrypted_invoke_transaction.encrypted_data.clone(),
            &symmetric_key,
            encrypted_invoke_transaction.nonce,
        );
        let decrypted_invoke_tx = String::from_utf8(decrypted_invoke_tx).unwrap();
        let decrypted_invoke_tx = decrypted_invoke_tx.trim_end_matches('\0');

        serde_json::from_str(&decrypted_invoke_tx).unwrap()
    }

    /// Delegate to decrypt encrypted invoke transaction
    pub async fn delegate_to_decrypt_encrypted_invoke_transaction(
        &self,
        encrypted_invoke_transaction: EncryptedInvokeTransaction,
        decryption_key: Option<String>,
    ) -> InvokeTransaction {
        println!("Decrypting encrypted invoke transaction... using external decryptor");

        let url = format!("ws://localhost:8080");
        let client = WsClientBuilder::default().build(&url).await.unwrap();

        let encrypted_invoke_transaction_json = json!(encrypted_invoke_transaction);

        let mut params = ObjectParams::new();
        if let Some(obj) = encrypted_invoke_transaction_json.as_object() {
            for (key, value) in obj {
                let _ = params.insert(key, value);
            }
        }

        let response: String = client.request("decrypt_transaction", params).await.unwrap();

        serde_json::from_str(response.as_str()).unwrap()
    }
}
