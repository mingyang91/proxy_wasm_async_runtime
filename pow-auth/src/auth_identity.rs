use secp256k1::Message;
use sha2::{Digest, Sha256};

pub struct AuthIdentity<'a, D> {
    pub_key: &'a secp256k1::PublicKey,
    data: D,
    signature: &'a secp256k1::ecdsa::Signature,
}

impl<'a, D> AuthIdentity<'a, D>
where
    D: Into<Message> + Clone,
{
    pub fn new(
        pub_key: &'a secp256k1::PublicKey,
        data: D,
        signature: &'a secp256k1::ecdsa::Signature,
    ) -> Self {
        Self {
            pub_key,
            data,
            signature,
        }
    }

    pub fn verify(&self) -> Result<(), secp256k1::Error> {
        let secp = secp256k1::Secp256k1::new();
        let msg: Message = self.data.clone().into();
        secp.verify_ecdsa(&msg, self.signature, self.pub_key)
    }
}

#[derive(Debug, Clone)]
pub struct AuthFactors<'a> {
    url: &'a str,
    timestamp: u64,
}

impl<'a> AuthFactors<'a> {
    pub fn new(url: &'a str, timestamp: u64) -> Self {
        Self { url, timestamp }
    }
}

impl From<AuthFactors<'_>> for Message {
    fn from(value: AuthFactors<'_>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(value.url.as_bytes());
        hasher.update(value.timestamp.to_be_bytes());
        let digest = hasher.finalize().into();
        Message::from_digest(digest)
    }
}

#[cfg(test)]
mod test {
    use hex_literal::hex;
    use secp256k1::{PublicKey, Secp256k1, SecretKey};

    use super::{AuthFactors, AuthIdentity};
    #[test]
    fn test() {
        let hex_secret = hex!("3f880ce0892ac66019804c80292d4e90a38aa70a9dabad3f4314bf050f492afc");
        let secret = SecretKey::from_slice(&hex_secret).unwrap();
        println!("{:?}", secret);
        let secp = Secp256k1::new();
        let pub_key = PublicKey::from_secret_key(&secp, &secret);

        let url = "/api/v1/hello";
        let timestamp = 1619823600;

        let factors = AuthFactors::new(url, timestamp);
        // let msg: Message = factors.into();
        // println!("{:?}", msg);
        let signature = secp.sign_ecdsa(&factors.clone().into(), &secret);
        let identity = AuthIdentity::new(&pub_key, factors, &signature);
        println!("{:?}", identity.verify());
    }
}
