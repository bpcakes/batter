use std::{future::Future, pin::Pin};
pub type Work = Pin<Box<dyn Future<Output = Result<u32, Failure>> + Send>>;
pub type Finish =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<(), batter::BoxError>> + Send>> + Send>;
pub type Acquire =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<Acquired, Failure>> + Send>> + Send>;
pub struct Acquired {
    pub work: Work,
    pub finish: Finish,
}
#[derive(Debug)]
pub enum Failure {
    Domain(&'static str),
    Registration(batter::RegistrationError),
}
impl From<batter::RegistrationError> for Failure {
    fn from(value: batter::RegistrationError) -> Self {
        Self::Registration(value)
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("consumer operation failed")
    }
}
impl std::error::Error for Failure {}
