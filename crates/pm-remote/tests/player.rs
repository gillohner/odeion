//! The other way through the tunnel: the node reaches the app's player away from home. A
//! controller serving a player takes the streams the node opens on its connection and pipes them
//! to the player's loopback listener; the node reaches it at its own loopback relay for that
//! controller's key, also after the controller connects again, and with a key kept across starts.

use pm_remote::{ClientTunnel, NodeTunnel, Relays, Route, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A "player": answers every connection by echoing what it reads, upper-cased.
async fn player() -> std::net::SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let up: Vec<u8> = buf[..n].to_ascii_uppercase();
                    if s.write_all(&up).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

async fn round_trip(relay: std::net::SocketAddr, msg: &[u8]) -> Vec<u8> {
    let mut s = tokio::net::TcpStream::connect(relay).await.unwrap();
    s.write_all(msg).await.unwrap();
    let mut buf = vec![0u8; msg.len()];
    tokio::time::timeout(std::time::Duration::from_secs(10), s.read_exact(&mut buf))
        .await
        .expect("an answer through the tunnel")
        .unwrap();
    buf
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_reaches_the_apps_player_through_the_tunnel() {
    // The node's HTTP side is not used here: any listener will do
    let api = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let node = NodeTunnel::start(SecretKey::generate(), &Relays::Off, api.local_addr().unwrap())
        .await
        .unwrap();
    let key = SecretKey::generate();
    let id = key.public();
    let app = ClientTunnel::start_with_key(node.addr(), &Relays::Off, Some(key.clone()))
        .await
        .unwrap();
    assert_eq!(app.connect().await.unwrap(), Route::Direct);
    app.serve_player(player().await).await;

    let relay = node.player_relay(app.id()).await.unwrap();
    assert!(relay.ip().is_loopback(), "{relay}");
    assert_eq!(
        node.player_relay(app.id()).await.unwrap(),
        relay,
        "one relay per app"
    );
    assert_eq!(round_trip(relay, b"hello").await, b"HELLO");
    // Several at once, each its own stream
    let (a, b) = tokio::join!(round_trip(relay, b"one"), round_trip(relay, b"two"));
    assert_eq!((a.as_slice(), b.as_slice()), (&b"ONE"[..], &b"TWO"[..]));

    // The app starts again with the key it kept: the same relay reaches its new connection
    app.close().await;
    let again = ClientTunnel::start_with_key(node.addr(), &Relays::Off, Some(key))
        .await
        .unwrap();
    again.connect().await.unwrap();
    again.serve_player(player().await).await;
    assert_eq!(again.id(), id, "the kept key: the node knows the app again");
    assert_eq!(node.player_relay(again.id()).await.unwrap(), relay);
    let mut ok = false;
    for _ in 0..50 {
        // The node learns the new connection as it is accepted
        if let Ok(Ok(b)) = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            let mut s = tokio::net::TcpStream::connect(relay).await?;
            s.write_all(b"back").await?;
            let mut buf = [0u8; 4];
            s.read_exact(&mut buf).await?;
            Ok::<_, std::io::Error>(buf)
        })
        .await
        {
            ok = &b == b"BACK";
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(ok, "the relay reaches the app after it connected again");

    // Another app (another key) is not reached through this relay
    let other = ClientTunnel::start(node.addr(), &Relays::Off).await.unwrap();
    other.connect().await.unwrap();
    assert_ne!(other.id(), again.id());
    node.close().await;
}
