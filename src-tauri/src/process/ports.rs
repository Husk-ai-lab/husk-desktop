use std::net::{Ipv4Addr, SocketAddr, TcpListener};

/// True when nothing is listening on 127.0.0.1:`port` (we can bind it ourselves).
pub fn is_free(port: u16) -> bool {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_busy_port() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(!is_free(port));
        drop(l);
        assert!(is_free(port));
    }
}
