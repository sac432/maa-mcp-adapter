pub type AdapterResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
