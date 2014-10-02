extern crate uuid;

use std::io::{TcpStream,TcpListener,Acceptor,Listener};
use std::sync::{Arc,RWLock};
use std::collections::HashMap;
use uuid::Uuid;

enum Method {
    Quit,
    Who,
    Name(String),
    Broadcast(String)
}

/*
 * A clients data structure. Essentially a shared hash map, so it is wrapped in an RWLock.
 *
 * Each client is assigned an id (Uuid) and stores a pair: Its broadcast sending channel and name.
 */
type Clients = Arc<RWLock<HashMap<Uuid, (Sender<(String, String)>, String)>>>;

/*
 * Input processing. This is written to take advantage of (a) Rust's iterators and (b) pattern matching.
 */
fn process_input(inp: &[u8]) -> Option<Method> {
    let mut it = inp.iter().peekable();
    let m: Vec<u8> = it.take_while(|&c| *c != b' '
                                   &&   *c != b'\r'
                                   &&   it.peek().map_or(false, |&c2| *c2 != b'\n'))
                       .map(|&c| c)
                       .collect();
    
    match m.into_ascii().into_string().as_slice() {
        "QUIT" => Some(Quit),
        "WHO"  => Some(Who),
        "NAME" => {
            let name: Vec<u8> = it.skip(5)
                                  .take_while(|&c| *c != b'\r' && it.peek().map_or(false, |&c2| *c2 != b'\n'))
                                  .map(|&c| c)
                                  .collect();
            if name.is_empty() { None } else { Some(Name(name.into_ascii().into_string())) }
        },
        "BROADCAST" => Some(Broadcast(it.skip(10)
                                        .take_while(|&c| *c != b'\r' && it.peek().map_or(false, |&c2| *c2 != b'\n'))
                                        .map(|&c| c)
                                        .collect::<Vec<u8>>()
                                        .into_ascii()
                                        .into_string())),
        _      => None
    }
}

/*
 * The client receives
 *
 * id: To be able to find itself in the client structure
 * stream: The TCP stream to read from
 * bcast: A receiver to receive broadcast messages
 */
fn handle_client(id: Uuid, mut stream: TcpStream, clients: Clients, bcast: Receiver<(String, String)>) {
    let mut buffer = [0u8, ..1024*16];
    let mut sc = stream.clone();
    let mut name = "".to_string();
    let (tx, rx) = channel();

    /*
     * Spawn reader
     *
     * 1) Parses received messages
     * 2) Quits when the (a) QUIT message is received or (b) a read error is detected
     */
    spawn(proc() {
        loop {
            match sc.read(buffer) {
                Ok(n)  => match process_input(buffer.slice(0,n-2)) {
                    Some(Quit) => {
                        tx.send(Quit);
                        break;
                    },
                    Some(m) => tx.send(m),
                    None    => {
                        sc.write(b"ERROR ").unwrap();
                        sc.write(buffer.slice(0, n)).unwrap();
                    }
                },
                Err(e) => {
                    println!("Received {}. Quitting.", e);
                    tx.send(Quit);
                    break;
                }
            }
        }
    });

    loop {
        select! {
            meth = rx.recv() => match meth {
                Quit => {
                    clients.write().pop(&id).unwrap();
                    drop(stream);
                    break;
                },
                Who => {
                    /* Write all user names to stream */
                    stream.write(b"NAMES").unwrap();
                    for &(_, ref name) in clients.read().values() {
                        stream.write(b" ").unwrap();
                        stream.write_str(name.as_slice()).unwrap();
                    }
                    stream.write(b"\r\n").unwrap();
                },
                Name(new_name) => {
                    name = new_name.clone();
                    let mut c = clients.write();
                    let (ch, _) = c.pop(&id).unwrap();
                    c.insert(id, (ch, new_name));
                },
                Broadcast(msg) => if name.is_empty() {
                    stream.write(b"NONAME\r\n").unwrap();
                } else {
                    for &(ref client, _) in clients.read().values() {
                        client.send((name.clone(), msg.clone()));;
                    }   
                }
            },
            (name, msg) = bcast.recv() => {
                stream.write(b"FROM ").unwrap();
                stream.write_str(name.as_slice()).unwrap();
                stream.write(b" ").unwrap();
                stream.write_str(msg.as_slice()).unwrap();
                stream.write(b"\r\n").unwrap();
            }
        }
    }
}

fn main() {
    let mut acpt = TcpListener::bind("127.0.0.1", 8090).listen().unwrap();
    acpt.set_timeout(None);

    let clients = Arc::new(RWLock::new(HashMap::new()));
    loop {
        match acpt.accept() {
            Ok(st) => {
                let (tx, rx) = channel();
                let id = Uuid::new_v4();
                clients.write().insert(id, (tx, "".to_string()));
                
                let clients_cln = clients.clone();
                spawn(proc() handle_client(id, st, clients_cln, rx))
            },
            Err(e) => {
                println!("{}", e);
            }
        }
    }
}
