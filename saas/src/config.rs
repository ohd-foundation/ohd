#[derive(Clone, Debug)]
pub struct Config {
    pub jwt_secret: String,
    pub token_ttl_days: i64,
}
