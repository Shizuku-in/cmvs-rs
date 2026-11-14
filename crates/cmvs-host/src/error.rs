use cmvs_core::runtime::RuntimeEvent;
use std::{error::Error, fmt};

/// Player failures with the original error retained as their source.
#[derive(Debug)]
pub enum PlayerError {
    /// Opening the game, font, save profile, or output services failed.
    Initialization(Box<dyn Error>),
    /// A VM step or frame update failed.
    Execution(Box<dyn Error>),
    /// Saving user preferences failed.
    Persistence(Box<dyn Error>),
    /// The VM reached a barrier or an unsupported command.
    Stopped(RuntimeEvent),
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initialization(error) => write!(f, "player initialization: {error}"),
            Self::Execution(error) => write!(f, "player execution: {error}"),
            Self::Persistence(error) => write!(f, "player preferences: {error}"),
            Self::Stopped(event) => write!(f, "player stopped: {event:?}"),
        }
    }
}

impl Error for PlayerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Initialization(error) | Self::Execution(error) | Self::Persistence(error) => {
                Some(error.as_ref())
            }
            Self::Stopped(_) => None,
        }
    }
}
