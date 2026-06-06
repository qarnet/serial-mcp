use thiserror::Error;

#[derive(Debug, Error)]
pub enum SerialError {
    #[error("Failed to open port: {0}")]
    OpenFailed(String),

    #[error("Port already open: {port}{owner}", owner = format_port_owner(.connection_id.as_deref(), .name.as_deref()))]
    PortAlreadyOpen {
        port: String,
        connection_id: Option<String>,
        name: Option<String>,
    },

    #[error("Port already opening: {0}")]
    PortAlreadyOpening(String),

    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    #[error("Connection closed: {0}")]
    ConnectionClosed(String),

    #[error("Invalid baud rate: {0}")]
    InvalidBaudRate(u32),

    #[error("Read timeout")]
    ReadTimeout,

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SerialError>;

impl From<serialport::Error> for SerialError {
    fn from(err: serialport::Error) -> Self {
        SerialError::IoError(std::io::Error::other(err.to_string()))
    }
}

fn format_port_owner(connection_id: Option<&str>, name: Option<&str>) -> String {
    match (connection_id, name) {
        (Some(id), Some(name)) => format!(" (owned by {id}, name={name:?})"),
        (Some(id), None) => format!(" (owned by {id})"),
        (None, Some(name)) => format!(" (name={name:?})"),
        (None, None) => String::new(),
    }
}
