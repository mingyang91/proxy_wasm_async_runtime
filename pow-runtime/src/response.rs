#[derive(Debug)]
pub struct Response {
    pub code: u32,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub trailers: Vec<(String, String)>,
}