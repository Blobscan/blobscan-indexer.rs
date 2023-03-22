use std::error;

pub type StdError = Box<dyn error::Error + Send + Sync>;
