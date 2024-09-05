use serde::{de::DeserializeOwned, Serialize};


pub trait Codec {
	type Error;
	fn encode(&self) -> Result<Vec<u8>, Self::Error>;
	fn decode(value: &[u8]) -> Result<Self, Self::Error> where Self: Sized;
}

#[cfg(feature = "serde_json")]
impl <V> Codec for V
where
	V: Serialize + DeserializeOwned 
{
	type Error = serde_json::Error;
	
	fn encode(&self) -> Result<Vec<u8>, Self::Error> {
			serde_json::to_vec(self)
	}
	
	fn decode(value: &[u8]) -> Result<Self, Self::Error> where Self: Sized {
			serde_json::from_slice(value)
	}
}

#[cfg(feature = "bincode")]
impl <V> Codec for V
where
	V: Serialize + DeserializeOwned 
{
	type Error = bincode::Error;
	
	fn encode(&self) -> Result<Vec<u8>, Self::Error> {
			bincode::serialize(self)
	}
	
	fn decode(value: &[u8]) -> Result<Self, Self::Error> where Self: Sized {
			bincode::deserialize(value)
	}
}
