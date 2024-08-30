use std::fmt::{Formatter, LowerHex};

pub type ByteArray32 = FixedByteArray<32>;

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub struct FixedByteArray<const N: usize>([u8; N]);

impl <const N: usize> From<&[u8; N]> for FixedByteArray<N> {
    fn from(bytes: &[u8; N]) -> Self {
        let mut result = [0; N];
        result.copy_from_slice(bytes);
        FixedByteArray(result)
    }
}

impl <const N: usize> TryFrom<&str> for FixedByteArray<N> {
    type Error = &'static str;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        if s.len() != N * 2 {
            return Err("invalid length");
        }
        let mut bytes = [0; N];
        for (i, item) in bytes.iter_mut().enumerate() {
            let start = i * 2;
            let end = start + 2;
            *item = u8::from_str_radix(&s[start..end], 16)
                .map_err(|_| "invalid hex")?;
        }
        Ok(FixedByteArray(bytes))
    }
}

impl <const N: usize> serde::Serialize for FixedByteArray<N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{:x}", self))
    }
}

impl <'de, const N: usize> serde::Deserialize<'de> for FixedByteArray<N> {
    fn deserialize<D>(deserializer: D) -> Result<FixedByteArray<N>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FixedByteArray::<N>::try_from(s.as_str()).map_err(serde::de::Error::custom)
    }
}

impl <const N: usize> LowerHex for FixedByteArray<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}
