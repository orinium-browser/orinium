#[derive(Debug)]
pub enum NetworkError {
    // Request / protocol
    InvalidUri,
    MissingHost,
    InvalidDnsName,

    // Transport
    ConnectionFailed,
    TlsFailed,
    Timeout,

    // HTTP
    HttpHandshakeFailed,
    HttpRequestFailed,
    HttpResponseFailed,
    TooManyRedirects,
    UnsupportedHttpVersion,

    // Infrastructure
    Disconnected,
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use NetworkError::*;
        let msg = match self {
            InvalidUri => "invalid URI",
            MissingHost => "URI has no host",
            InvalidDnsName => "invalid DNS name",

            ConnectionFailed => "connection failed",
            TlsFailed => "TLS handshake failed",
            Timeout => "network timeout",

            HttpHandshakeFailed => "HTTP handshake failed",
            HttpRequestFailed => "HTTP request failed",
            HttpResponseFailed => "HTTP response failed",
            TooManyRedirects => "too many redirects",
            UnsupportedHttpVersion => "unsupported HTTP version",

            Disconnected => "network subsystem disconnected",
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for NetworkError {}
