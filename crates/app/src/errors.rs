use esp_radio::{wifi::WifiError, InitializationError};

#[derive(defmt::Format)]
pub enum RuntimeError {
    None,
    WifiError(WifiError),
    InitializationError(InitializationError),
}

impl From<WifiError> for RuntimeError {
    fn from(error: WifiError) -> Self {
        RuntimeError::WifiError(error)
    }
}

impl From<()> for RuntimeError {
    fn from(_: ()) -> Self {
        RuntimeError::None
    }
}

impl From<InitializationError> for RuntimeError {
    fn from(error: InitializationError) -> Self {
        RuntimeError::InitializationError(error)
    }
}
