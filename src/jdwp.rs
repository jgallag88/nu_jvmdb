use std::cell::{Cell, RefCell};
use std::convert::TryInto;
use std::net::ToSocketAddrs;
use std::net::TcpStream;
use std::io::Result;
use std::io::{Cursor, Read, Write};
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

        let len = data.len() + 11; // 11 is size of header
        stream.write_u32::<BigEndian>(len.try_into().unwrap())?;
        stream.write_u32::<BigEndian>(id)?;
        stream.write_u8(0)?; // Flags
        stream.write_u8(command_set)?;
        stream.write_u8(command)?;
        stream.write_all(data)?;

        let len = stream.read_u32::<BigEndian>()? - 11; // 11 is size of header
        let _id = stream.read_u32::<BigEndian>()?; // TODO check that id is what we expect
        let flags = stream.read_u8()?; // TODO check response flag
        let error_code = stream.read_u16::<BigEndian>()?;
        let mut buf = vec![0; len as usize];
        stream.read_exact(&mut buf)?;
        Ok(buf)
    }
}


trait Serialize {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()>;
    fn deserialize<R: Read>(reader: &mut R) -> Result<Self>
        where Self: std::marker::Sized;
}

impl Serialize for u8 {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()> {
        writer.write_u8(self)
    }
    fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        reader.read_u8()
    }
}

impl Serialize for u16 {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()> {
        writer.write_u16::<BigEndian>(self)
    }
    fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        reader.read_u16::<BigEndian>()
    }
}

impl Serialize for u32 {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()> {
        writer.write_u32::<BigEndian>(self)
    }
    fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        reader.read_u32::<BigEndian>()
    }
}

impl Serialize for String {
    fn serialize<W: Write>(self, writer: &mut W) -> Result<()> {
        let utf8 = self.as_bytes();
        writer.write_u32::<BigEndian>(utf8.len().try_into().unwrap())?;
        writer.write_all(utf8);
        Ok(())
    }
    
    fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        let str_len = reader.read_u32::<BigEndian>()?;
    
        let mut buf = vec![0; str_len as usize];
        reader.read_exact(&mut buf)?;
        // TODO handle utf8 conversion errors, which will involve changing return
        // type (or maybe using lossy conversion?)
        Ok(String::from_utf8(buf).unwrap())
    }
}


macro_rules! command {
    ( $command:ident, $id:expr;
      $( $arg:ident: $arg_ty:ty ),*;
      $resp_name:ident;
      $( $resp_val:ident: $resp_val_ty:ty ),*
    ) => {

        #[derive(Debug)]
        pub struct $resp_name {
            $(
                $resp_val: $resp_val_ty,
            )*
        }

        pub fn $command(conn: JdwpConnection $(, $arg: $arg_ty )* ) -> Result<$resp_name> {
            let mut buf = vec![];
            $(
                $arg.serialize(&mut buf);
            )*
            let mut resp_buf = &mut Cursor::new(conn.execute_cmd(1, $id, &buf)?);

            Ok($resp_name {
                $(
                    $resp_val: Serialize::deserialize(resp_buf)?,
                )*
            })
        }
    };
}

command! {
    version, 1; ;
    VersionReply;
        description: String,
        jdwpMajor: u32, // TODO this should be i32
        jdwpMinor: u32, // TODO this should be i32
        vmVersion: String,
        vmName: String
}

macro_rules! command_set {
    ( $command_set:ident, $set_id:expr; ) => {
        struct $command_set {}
    };
}

command_set!{ Foo, 23; }
