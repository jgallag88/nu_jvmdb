use std::collections::HashMap;
use std::env;
use std::process::Command;
use tokio::prelude::*;
use tokio::sync::mpsc;
use tokio::net::{TcpStream, TcpListener};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

const JDWP_INIT_MSG: &'static [u8] = b"JDWP-Handshake";

// TODO call this Msg?
struct Reply {
    len: u32,
    id: u32,
    rest: Vec<u8>,
}

// A msg which has been received from a client and needs to be forwarded to the
// JVM.
struct IncomingCmd {
    len: u32,
    id: u32,
    rest: Vec<u8>,
    reply_chan: mpsc::Sender<Reply>,
}

async fn read_client(mut socket: OwnedReadHalf,
                     chan: mpsc::Sender<IncomingCmd>,
                     reply_chan: mpsc::Sender<Reply>) -> io::Result<()> {
    loop {
        let len = socket.read_u32().await?;
        let id = socket.read_u32().await?;
        let mut rest = vec![0; len as usize - 8]; // TODO check size before cast
        socket.read_exact(&mut rest).await?;

        chan.send(IncomingCmd {
            len: len,
            id: id,
            rest: rest,
            reply_chan: reply_chan.clone(),
        }).await; // TODO check result ?
    }
}

async fn write_client(mut socket: OwnedWriteHalf,
                      mut chan: mpsc::Receiver<Reply>) -> io::Result<()> {

    while let Some(reply) = chan.recv().await {
        socket.write_u32(reply.len).await?;
        socket.write_u32(reply.id).await?;
        socket.write_all(&reply.rest).await?;
    }

    Ok(())
}

// Take commands that have been received from a client and forward them to the
// debug server (the JVM).
async fn write_server(mut socket: OwnedWriteHalf,
                      mut chan_incoming: mpsc::Receiver<IncomingCmd>,
                      chan_outstanding: mpsc::Sender<OutstandingCmd>) -> io::Result<()> {

    let mut proxy_id = 0;
    while let Some(cmd) = chan_incoming.recv().await {
        // Forward the info about the outstanding cmd before issuing it, to make
        // sure that the task handling the read end of the socket knows about the
        // cmd before the corresponding reply arrives.
        chan_outstanding.send(OutstandingCmd {
            client_id: cmd.id,
            proxy_id: proxy_id,
            reply_chan: cmd.reply_chan,
        }).await;

        socket.write_u32(cmd.len).await?;
        socket.write_u32(proxy_id).await?;
        socket.write_all(&cmd.rest).await?;

        proxy_id += 1;
    }

    Ok(())
}

struct OutstandingCmd {
    client_id: u32,
    proxy_id: u32,
    reply_chan: mpsc::Sender<Reply>,
}

async fn read_server(mut socket: OwnedReadHalf,
                     mut chan: mpsc::Receiver<OutstandingCmd>) -> io::Result<()> {

    // TODO rename?
    let mut id_map: HashMap<u32, (u32, mpsc::Sender<Reply>)> = HashMap::new();

    loop {
        // TODO deduplicate wrt read_client
        println!("1");
        let len = socket.read_u32().await?;
        println!("{}", len);
        let id = socket.read_u32().await?;
        println!("{}", id);
        let mut rest = vec![0; len as usize - 8]; // TODO check size before cast
        socket.read_exact(&mut rest).await?;
        println!("{:?}", rest);

        // The info associated with this reply should be waiting in the channel,
        // if we haven't already receivied it.
        while let Ok(outstanding) = chan.try_recv() {
            let old_val = id_map.insert(outstanding.proxy_id,
                                        (outstanding.client_id, outstanding.reply_chan));
            assert!(old_val.is_none()); // when it stabilizes, can use unwrap_none().
        }

        match id_map.remove(&id) {
            // Forward the reply to the relevant client writer task, replacing
            // the proxy id with the original id supplied by the client.
            Some((client_id, reply_chan)) => { reply_chan.send(
                Reply {
                    len: len,
                    id: client_id,
                    rest: rest,
                }).await;
            },
            None => { panic!("didn't find id!"); } // TODO should this be fatal?
        }
    }
}

// TODO figure out how to clean up half-open connections on termination / error
async fn handle_client(mut socket: TcpStream,
                       cmd_tx: mpsc::Sender<IncomingCmd>) -> io::Result<()> {

    // Protocol initialization
    let mut buf = [0u8; JDWP_INIT_MSG.len()];
    let n = socket.read_exact(&mut buf).await?;
    // TODO check that what we got is what we expect
    socket.write_all(&JDWP_INIT_MSG).await?;
    socket.flush();
    // TODO do we need to flush?
    
    let (read_side, write_side) = socket.into_split();
    let (reply_tx, reply_rx) = mpsc::channel(100);

    tokio::spawn(async move {
        read_client(read_side, cmd_tx, reply_tx).await
    });

    tokio::spawn(async move {
        write_client(write_side, reply_rx).await
    });

    Ok(())
}

/*
 * ====>  TCP stream
 * ---->  mpsc channel
 *
 *                                                 Proxy                                          JVM
 *   client        ----------------------------------------------------------------------      ----------
 * ----------      |    ---------------------                ---------------------      |      |        |
 * |        | =====|==> | read_client task  | -------------> | write_server task | =====|====> |        |
 * |        |      |    ---------------------   incoming |   ---------------------      |      |        |
 * |        |      |                                     |               |              |      |        |
 * |        |      |    ---------------------            |               |              |      |        |
 * |        | <====|=== | write_client task | <-----     |               |              |      |        |
 * ----------      |    ---------------------      |     |               |              |      |        |
 *                 |                               |     |               |              |      |        |
 *   client        |                               |     |               | outstanding  |      |        |
 * ----------      |    ---------------------      |     |               |              |      |        |
 * |        | =====|==> | read_client task  | ------------               |              |      |        |
 * |        |      |    ---------------------      |                     |              |      |        |
 * |        |      |                               |                     V              |      |        |
 * |        |      |    ---------------------      |          ---------------------     |      |        |
 * |        | <====|=== | write_client task | <-------------- | read_server task  | <===|===== |        |
 * ----------      |    ---------------------    outgoing     ---------------------     |      |        |
 *                 |                                                                    |      |        |
 *     .           |              .                                                     |      |        |
 *     .           |              .                                                     |      |        |
 *     .           |              .                                                     |      |        |
 *                 |                                                                    |      |        |
 *                 ----------------------------------------------------------------------      ----------
 */

#[tokio::main]
async fn main() -> io::Result<()> {

    // Open connection to JVM, do protocol initialization
    let mut socket = TcpStream::connect("localhost:5005").await?;
    socket.write_all(&JDWP_INIT_MSG).await?;
    socket.flush();
    let mut buf = [0u8; JDWP_INIT_MSG.len()];
    let n = socket.read_exact(&mut buf).await?;
    // TODO check that what we got is what we expect

    let (incoming_tx, incoming_rx) = mpsc::channel(500);
    let (outstanding_tx, outstanding_rx) = mpsc::channel(500);
    let (read_side, write_side) = socket.into_split();
    tokio::spawn(async move {
        write_server(write_side, incoming_rx, outstanding_tx).await
    });
    tokio::spawn(async move {
        read_server(read_side, outstanding_rx).await
    });

    let listener = TcpListener::bind("localhost:1234").await?;

    env::set_var("JVMDBG_PROXY_PORT", "1234");
    Command::new("nu").spawn()?;

    loop {
        let (socket, _) = listener.accept().await?;
        println!("Accepted conn");
        let incoming_tx = incoming_tx.clone();

        tokio::spawn(async move {
            handle_client(socket, incoming_tx).await
        });
    }
}
