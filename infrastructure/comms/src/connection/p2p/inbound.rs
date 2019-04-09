//  Copyright 2019 The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::connection::{
    NetAddress,
    net_address::ToZmqEndpoint,
    Context,
    types::SocketType,
    Result,
    ConnectionError,
    message::FrameSet,
    message::Frame,
};

use std::error::Error;

pub struct InboundConnection<'a> {
    context: &'a Context,
    recv_hwm: Option<i32>,
    send_hwm: Option<i32>,
    linger: Option<i32>,
    curve_secret_key: Option<[u8;32]>,
    max_message_size: Option<i64>,
}

impl<'a> InboundConnection<'a> {
    pub fn new(context: &'a Context) -> Self {
        Self {
            context,
            recv_hwm: None,
            send_hwm: None,
            linger: Some(200),
            curve_secret_key: None,
            max_message_size: None,
        }
    }

    pub fn set_recv_hwm(&mut self, v: i32) -> &mut Self {
        self.recv_hwm = Some(v);
        self
    }

    pub fn set_send_hwm(&mut self, v: i32) -> &mut Self {
        self.send_hwm = Some(v);
        self
    }

    pub fn set_linger(&mut self, v: i32) -> &mut Self {
        self.linger = Some(v);
        self
    }

    pub fn set_max_message_size(&mut self, v: i64) -> &mut Self {
        self.max_message_size = Some(v);
        self
    }

    pub fn set_curve_secret_key(&mut self, v: [u8;32]) -> &mut Self {
        self.curve_secret_key = Some(v);
        self
    }

    pub fn bind(&self, addr: &str) -> Result<BoundInboundConnection> {
        let socket = self.context.socket(SocketType::Router).unwrap();

        socket.set_router_mandatory(true);

        if let Some(v) = self.recv_hwm {
            socket.set_rcvhwm(v);
        }

        if let Some(v) = self.send_hwm {
            socket.set_sndhwm(v);
        }

        if let Some(v) = self.max_message_size {
            socket.set_maxmsgsize(v);
        }

        if let Some(v) = self.linger {
            socket.set_linger(v);
        }

        if let Some(ref v) = self.curve_secret_key {
            socket.set_curve_server(true);
            socket.set_curve_secretkey(v);
        }

        socket.bind(addr)
            .map_err(|e| ConnectionError::SocketError(format!("Failed to bind inbound socket: {}", e)))?;

        Ok(BoundInboundConnection {
            context: self.context.clone(),
            socket,
        })
    }
}

pub struct BoundInboundConnection {
    context: Context,
    socket: zmq::Socket,
}

impl BoundInboundConnection {
    pub fn receive(&self, timeout: i64) -> Result<FrameSet> {
        match self.socket.poll(zmq::POLLIN, timeout) {
            Ok(rc) => {
                match rc {
                    // Internal error when polling connection
                    -1 => Err(ConnectionError::SocketError("Failed to poll socket".to_string())),
                    // Nothing to receive
                    0 => Err(ConnectionError::Timeout),
                    // Ready to receive
                    len => {
                        println!("{}", len);
                        self.receive_multipart()
                    }
                }
            }

            Err(e) => Err(ConnectionError::SocketError(format!("Failed to poll on inbound connection: {}", e))),
        }
    }

    pub fn send(&self, frames: FrameSet) -> Result<()> {
        for (i, frame) in frames.iter().enumerate() {
            let flags = if i < frames.len() - 1 { zmq::SNDMORE } else { 0 };
            self.socket.send(frame.as_slice(), flags)
                .map_err(|e| ConnectionError::SocketError(format!("Failed to send from inbound connection: {}", e)))?;
        }
        Ok(())
    }

    fn receive_multipart(&self) -> Result<FrameSet> {
        self.socket.recv_multipart(0)
            .map_err(|e| ConnectionError::SocketError(format!("{}", e)))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::{OsRng, thread_rng, Rng, RngCore, distributions::Alphanumeric};
    use std::{thread, iter, sync::mpsc::{channel, Receiver}};
    use crate::connection::types::SocketType;
    use std::time::Duration;

    fn make_inproc_addr() -> String {
        let mut rng = thread_rng();
        let rand_str: String = iter::repeat(())
            .map(|_| rng.sample(Alphanumeric))
            .take(5)
            .collect();
        format!("inproc://{}", rand_str)
    }

    fn send_to_address(ctx: &Context, addr: String, identity: String, msgs: FrameSet, public_key: Option<[u8;32]>) -> Receiver<()> {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let socket = ctx.socket(SocketType::Dealer).unwrap();
            socket.set_identity(identity.as_bytes()).unwrap();

            let keypair = zmq::CurveKeyPair::new().unwrap();
            if let Some(ref public_key) = public_key {
                socket.set_curve_serverkey(public_key);
                socket.set_curve_publickey(&keypair.public_key);
                socket.set_curve_secretkey(&keypair.secret_key);
            }
            socket.set_linger(1000);
            socket.connect(addr.as_str()).unwrap();
            socket.send_multipart(msgs.iter().map(|s| s.as_slice()).collect::<Vec<&[u8]>>().as_slice(), 0).unwrap();
            tx.send(()).unwrap();
            socket.recv_bytes(0).unwrap();
        });
        rx
    }

    #[test]
    fn receive_timeout() {
        let ctx = Context::new();

        let addr = make_inproc_addr();

        let conn = InboundConnection::new(&ctx)
            .bind(&addr).unwrap();

        let result = conn.receive(1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            ConnectionError::Timeout => {}
            _ => panic!("Unexpected error type: {:?}", err),
        }
    }

    #[test]
    fn receive_inproc() {
        let ctx = Context::new();

        let addr = make_inproc_addr();

        let keypair = zmq::CurveKeyPair::new().unwrap();

        let conn = InboundConnection::new(&ctx)
            .bind(&addr)
            .unwrap();

        let signal = send_to_address(&ctx, addr, "identity".to_string(), vec![
            "Just".as_bytes().to_vec(),
            "Three".as_bytes().to_vec(),
            "Messages".as_bytes().to_vec(),
        ], None);

        let frames = conn.receive(1000).unwrap();
        assert_eq!(frames.len(), 4);
        assert_eq!("identity".as_bytes(), frames[0].as_slice());
        assert_eq!("Just".as_bytes(), frames[1].as_slice());
        assert_eq!("Three".as_bytes(), frames[2].as_slice());
        assert_eq!("Messages".as_bytes(), frames[3].as_slice());
    }

    #[test]
    fn receive_encrypted_tcp() {
        let ctx = Context::new();

        let addr = "tcp://127.0.0.1:33333";

        let keypair = zmq::CurveKeyPair::new().unwrap();

        let conn = InboundConnection::new(&ctx)
            .set_curve_secret_key(keypair.secret_key)
            .bind(addr)
            .unwrap();

        let signal = send_to_address(&ctx, addr.to_string(), "identity".to_string(), vec![
            (0..1000000).map(|i| i as u8).collect::<Vec<_>>(),
        ], Some(keypair.public_key));

        let frames = conn.receive(1000).unwrap();
        assert_eq!(frames.len(), 2);
    }
}
