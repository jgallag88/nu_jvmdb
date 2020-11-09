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
    field_id_size: u8,
    method_id_size: u8,
    object_id_size: u8,
    reference_type_id_size: u8,
    frame_id_size: u8
}

impl JdwpConnection {
    pub fn new<A: ToSocketAddrs>(jvm_debug_addr: A) -> Result<Self> {
        let mut stream = TcpStream::connect("localhost:5005")?;
        stream.write_all(b"JDWP-Handshake")?;
        // TODO do we need to flush?
        let mut buf = [0; 128];
        let n = stream.read(&mut buf)?;
        // TODO check that response is what we expect, correct len, etc.
        

        let mut conn = JdwpConnection {
            stream: RefCell::new(stream),
            next_id: Cell::new(0),
            // Unfortunately, the JDWP protocol isn't defined entirely
            // statically. After establishing a connection, the client must
            // query the JVM to figure out the size of certain fields that
            // will be sent/recieved in future messages. Set the sizes to zero,
            // but fill them in before we hand the struct to the caller.
            field_id_size: 0,
            method_id_size: 0,
            object_id_size: 0,
            reference_type_id_size: 0,
            frame_id_size: 0,
        };

        let id_sizes = { id_sizes(&conn)? };
        // TODO check sizes
        conn.field_id_size = id_sizes.field_id_size.try_into().unwrap();
        conn.method_id_size = id_sizes.method_id_size.try_into().unwrap();
        conn.object_id_size = id_sizes.object_id_size.try_into().unwrap();
        conn.reference_type_id_size = id_sizes.reference_type_id_size.try_into().unwrap();
        conn.frame_id_size = id_sizes.frame_id_size.try_into().unwrap();

        Ok(conn)
    }

    fn execute_cmd(&self, command_set: u8, command: u8,  data: &[u8]) -> Result<Vec<u8>> {
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
    ( command_fn: $command:ident;
      command_id: $id:expr;
      args: {
          $( $arg:ident: $arg_ty:ty ),*
      }
      response_type: $resp_name:ident {
          $( $resp_val:ident: $resp_val_ty:ty ),*
      }
    ) => {

        #[derive(Debug)]
        pub struct $resp_name {
            $(
                $resp_val: $resp_val_ty,
            )*
        }

        pub fn $command(conn: &JdwpConnection $(, $arg: $arg_ty )* ) -> Result<$resp_name> {
            let mut buf = vec![];
            $(
                $arg.serialize(&mut buf)?;
            )*
            let mut resp_buf = Cursor::new(conn.execute_cmd(1, $id, &buf)?);

            Ok($resp_name {
                $(
                    $resp_val: Serialize::deserialize(&mut resp_buf)?,
                )*
            })
        }
    };
}

command! {
    command_fn: version;
    command_id: 1;
    args: {}
    response_type: VersionReply {
        description: String,
        jdwp_major: u32, // TODO this should be i32
        jdwp_minor: u32, // TODO this should be i32
        vm_version: String,
        vm_name: String
    }
}
command! {
    command_fn: id_sizes;
    command_id: 7;
    args: {}
    response_type: IdSizesReply {
        field_id_size: u32, // TODO this should be i32
        method_id_size: u32, // TODO this should be i32
        object_id_size: u32, // TODO this should be i32
        reference_type_id_size: u32, // TODO this should be i32
        frame_id_size: u32 // TODO this should be i32
    }
}
command! {
    command_fn: exit;
    command_id: 10;
    args: {
        exit_code: u32 // TODO this should be i32
    }
    response_type: ExitReply {}
}

macro_rules! command_set {
    ( $command_set:ident, $set_id:expr; ) => {
        struct $command_set {}
    };
}

command_set!{ Foo, 23; }
