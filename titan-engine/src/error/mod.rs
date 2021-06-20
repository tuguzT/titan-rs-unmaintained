use std::error::Error as StdError;
use std::fmt::{self, Debug, Display, Formatter};

use ash::vk;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Graphics {
        result: vk::Result,
    },
    Other {
        message: String,
        source: Option<Box<dyn StdError>>,
    },
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Graphics { result } => Display::fmt(result, f),
            Self::Other { message, .. } => write!(f, "{}", message),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Graphics { result: code } => Some(code),
            Self::Other { source, .. } => source.as_ref().map(|error| error.as_ref()),
        }
    }
}

impl From<vk::Result> for Error {
    fn from(result: vk::Result) -> Self {
        Self::Graphics { result }
    }
}
