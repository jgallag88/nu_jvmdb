use std::cell::{Cell, RefCell};
use std::convert::TryInto;
use std::net::ToSocketAddrs;
use std::net::TcpStream;
use std::io::Result;
use std::io::{Read, Write};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

pub struct JdwpConnection {
    stream: RefCell<TcpStream>, // TODO wrap in buffered stream?
    next_id: Cell<u32>,
}

impl JdwpConnection {
    pub fn new<A: ToSocketAddrs>(jvm_debug_addr: A) -> Result<Self> {
        let mut stream = TcpStream::connect("localhost:5005")?;
        stream.write_all(b"JDWP-Handshake")?;
        // TODO do we need to flush?
        let mut buf = [0; 128];
        let n = stream.read(&mut buf)?;
        // TODO check that response is what we expect, correct len, etc.

        Ok(JdwpConnection {
            stream: RefCell::new(stream),
            next_id: Cell::new(0),
        })
    }

    fn execute_cmd(self, command_set: u8, command: u8,  data: &[u8]) -> Result<Vec<u8>> {
        let mut stream = &mut *self.stream.borrow_mut();
        let id = self.next_id.get();
        self.next_id.set(id+1);

        send_command(stream, 1, 1, id, &[])?;

        let len = data.len() + 11; // 11 is size of header
        stream.write_u32::<BigEndian>(len.try_into().unwrap())?;
        stream.write_u32::<BigEndian>(id)?;
        stream.write_u8(0)?; // Flags
        stream.write_u8(command_set)?;
        stream.write_u8(command)?;
        stream.write_all(data)?;

        let len = stream.read_u32::<BigEndian>()? - 11; // 11 is size of header
        let id = stream.read_u32::<BigEndian>()?; // TODO check that id is what we expect
        let flags = stream.read_u8()?;
        let error_code = stream.read_u16::<BigEndian>()?;
        let mut buf = vec![0; len as usize];
        stream.read_exact(&mut buf)?;
        Ok(buf)
    }
}



fn send_command(writer: &mut dyn Write,
               command_set: u8,
               command: u8,
               id: u32,
               data: &[u8]) -> Result<()> {

    let len = data.len() + 11; // 11 is size of header
    writer.write_u32::<BigEndian>(len.try_into().unwrap())?;
    writer.write_u32::<BigEndian>(id)?;
    writer.write_u8(0)?; // Flags
    writer.write_u8(command_set)?;
    writer.write_u8(command)?;
    writer.write_all(data)?;

    Ok(())
}

fn recv_reply(reader: &mut dyn Read) -> Result<Vec<u8>> {
    let len = reader.read_u32::<BigEndian>()? - 11; // 11 is size of header
    let id = reader.read_u32::<BigEndian>()?; // TODO check that id is what we expect
    let flags = reader.read_u8()?;
    let error_code = reader.read_u16::<BigEndian>()?;

    let mut buf = vec![0; len as usize];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}


fn serialize_string<W: Write>(writer: &mut W, s: &str) -> Result<()> {
    let utf8 = s.as_bytes();
    writer.write_u32::<BigEndian>(utf8.len().try_into().unwrap())?;
    writer.write_all(utf8);
    Ok(())
}

fn deserialize_string<R: Read>(reader: &mut R) -> Result<String> {
    let str_len = reader.read_u32::<BigEndian>()?;

    let mut buf = vec![0; str_len as usize];
    reader.read_exact(&mut buf)?;
    // TODO handle utf8 conversion errors, which will involve changing return
    // type (or maybe using lossy conversion?)
    Ok(String::from_utf8(buf).unwrap())
}

trait Serialize {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()>;
    //fn deserialize<R: Read>(reader: &mut R) -> Result<Self>;
}

impl Serialize for u8 {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()> {
        writer.write_u8(self)
    }
}

impl Serialize for u32 {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()> {
        writer.write_u32::<BigEndian>(self)
    }
}

macro_rules! command {
    ( $command:ident, $id:expr; $( $arg:ident: $arg_ty:ty ),* ) => {
        fn $command(conn: JdwpConnection $(, $arg: $arg_ty )* ) {
            let mut buf = vec![];
            $(
                $arg.serialize(&mut buf);
            )*
            conn.execute_cmd(1, $id, &buf);
        }
    };
}

command! {
    foo, 23; blah: u32, asdf: u8
}

macro_rules! command_set {
    ( $command_set:ident, $set_id:expr; ) => {
        struct $command_set {}
    };
}

command_set!{ Foo, 23; }
