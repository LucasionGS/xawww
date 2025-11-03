include!(concat!(env!("OUT_DIR"), "/wayland_protocols.rs"));

use waybackend::{Waybackend, objman::ObjectManager, wire::Receiver};

use crate::WaylandObject;

pub fn connect() -> (Waybackend, ObjectManager<WaylandObject>, Receiver) {
    use rustix::fd::{FromRawFd, OwnedFd};
    use rustix::net::AddressFamily;

    if let Some(txt) = common::getenv(c"WAYLAND_SOCKET") {
        // We should connect to the provided WAYLAND_SOCKET
        let fd = txt.to_str().map(str::parse::<i32>).unwrap().unwrap();

        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let socket_addr = rustix::net::getsockname(&fd).expect("failed to getsocketname");
        if socket_addr.address_family() == AddressFamily::UNIX {
            unsafe { waybackend::connect_from_fd(WaylandObject::Display, fd) }
        } else {
            panic!(
                "Socket in WAYLAND_SOCKET has wrong family: {:?}",
                socket_addr.address_family()
            );
        }
    } else {
        let socket_name = common::getenv(c"WAYLAND_DISPLAY").unwrap_or_else(|| {
            log::warn!("WAYLAND_DISPLAY is not set! Defaulting to wayland-0");
            c"wayland-0"
        });

        let unix_addr = if socket_name.to_bytes()[0] == b'/' {
            rustix::net::SocketAddrUnix::new(socket_name).unwrap()
        } else {
            let mut socket_fullpath = Vec::new();
            match common::getenv(c"XDG_RUNTIME_DIR") {
                Some(socket_path) => {
                    socket_fullpath.extend_from_slice(socket_path.to_bytes());
                    socket_fullpath.push(b'/');
                }
                None => {
                    log::warn!("XDG_RUNTIME_DIR is not set! Defaulting to /run/user/UID");
                    let uid = rustix::process::getuid();
                    socket_fullpath.extend_from_slice(b"/run/user/");
                    socket_fullpath.extend_from_slice(uid.as_raw().to_string().as_bytes());
                    socket_fullpath.push(b'/');
                }
            }
            socket_fullpath.extend_from_slice(socket_name.to_bytes());
            rustix::net::SocketAddrUnix::new(socket_fullpath.as_slice()).unwrap()
        };

        let socket = rustix::net::socket_with(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::STREAM,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("failed to create socket");

        waybackend::connect_to(WaylandObject::Display, socket, &unix_addr)
            .expect("failed to connect to socket")
    }
}
