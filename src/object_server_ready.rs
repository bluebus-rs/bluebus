//! Works around a start-up race of zbus' object server.
//!
//! `Connection::object_server()` spawns the task that receives incoming
//! method calls on the connection's executor and returns immediately. A
//! method call that arrives before that task has subscribed is silently
//! dropped. BlueZ calls back into the application (`GetAll` on an
//! advertisement, `GetManagedObjects` on a GATT application, agent methods)
//! right after `Register*`, so an unlucky first registration waited forever.
//!
//! The dispatcher subscribes for method calls locally (no bus round trip)
//! during its first polls. Running a few no-op tasks on the same FIFO
//! executor and awaiting them guarantees those polls have happened. Calling
//! ourselves over the bus is not an option: the system bus policy forbids it.

/// Starts the object server of `conn` (if needed) and waits until its
/// dispatcher has had the chance to subscribe.
pub(crate) async fn ensure_object_server_ready(conn: &zbus::Connection) -> zbus::Result<()> {
    let _ = conn.object_server();
    for _ in 0..4 {
        let _ = conn
            .executor()
            .spawn(async {}, "bluebus-object-server-probe")
            .await;
    }
    Ok(())
}
