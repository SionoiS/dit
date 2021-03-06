use crate::Address;

use serde::{Deserialize, Serialize};

use secp256k1::recover;
use secp256k1::{Message, RecoveryId, Signature};

/// Generic crypto-signed message.
#[derive(Serialize, Deserialize, Debug)]
pub struct SignedMessage<T>
where
    T: Serialize,
{
    pub address: Address,

    pub data: T,

    pub signature: Vec<u8>, // Should be [u8; 65] but serde can't deal with big arrays.
}

impl<T> SignedMessage<T>
where
    T: Serialize,
{
    pub fn verify(&self) -> bool {
        if self.signature.len() != 65 {
            return false;
        }

        let message = serde_json::to_vec(&self.data).expect("Cannot Serialize");

        let mut eth_message =
            format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
        eth_message.extend_from_slice(&message);

        let hash = keccak256(&eth_message);

        let msg = Message::parse_slice(&hash).expect("Invalid Message");
        let sig = Signature::parse_slice(&self.signature[0..64]).expect("Invalid Signature");
        let rec_id = RecoveryId::parse_rpc(self.signature[64]).expect("Invalid Recovery Id");

        let public_key = match recover(&msg, &sig, &rec_id) {
            Ok(data) => data.serialize(),
            Err(_) => return false,
        };

        // The public key returned is 65 bytes long, that is because it is prefixed by `0x04` to indicate an uncompressed public key.
        let hash = keccak256(&public_key[1..]);

        // The public address is defined as the low 20 bytes of the keccak hash of the public key.
        hash[12..] == self.address
    }
}

/// Compute the Keccak-256 hash of input bytes.
fn keccak256(bytes: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut output);
    output
}
