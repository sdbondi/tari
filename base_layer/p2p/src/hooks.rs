//  Copyright 2020, The Tari Project
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

use futures::{
    future::{BoxFuture, FutureExt},
    Future,
};
use std::sync::Arc;
use tari_comms::{
    peer_manager::Peer,
    protocol::{ProtocolExtension, ProtocolExtensions},
    NodeIdentity,
};
use tari_comms_dht::outbound::OutboundMessageRequester;

pub type InitializationHookError = anyhow::Error;

#[derive(Default)]
pub struct P2pInitializationHooks {
    before_build: Vec<BoxedAsyncHook<()>>,
    before_spawn: Vec<BoxedAsyncHook<BeforeSpawnContext>>,
}

impl P2pInitializationHooks {
    /// Create a new `P2pInitializationHooks`
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a `before_build` hook.
    ///
    /// This occurs just before the comms stack has been built.
    pub fn before_build<T: AsyncHook<()> + Sized + 'static>(&mut self, hook: T) -> &mut Self {
        self.before_build.push(hook.boxed());
        self
    }

    /// Add a `before_spawn` hook.
    ///
    /// This hook occurs after comms has been built but before it's services have been spawned.
    /// Useful for initializing other components that depend on Comms or DHT services (e.g. PeerManager, Outbound
    /// messaging etc).
    pub fn before_spawn<T: AsyncHook<BeforeSpawnContext> + Sized + 'static>(&mut self, hook: T) -> &mut Self {
        self.before_spawn.push(hook.boxed());
        self
    }

    /// Adds a before_spawn hook for an `FnOnce` closure.
    /// If your closure does not capture state from it's environment, `before_spawn` can be used.
    pub fn before_spawn_fn<F, Fut>(&mut self, hook: F) -> &mut Self
    where
        F: FnOnce(BeforeSpawnContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<BeforeSpawnContext, InitializationHookError>> + Send + 'static,
    {
        self.before_spawn(FnWrapper::new(hook));
        self
    }

    pub(crate) async fn call_before_build(&mut self) -> Result<(), InitializationHookError> {
        for hook in &mut self.before_build {
            hook.call(()).await?
        }
        Ok(())
    }

    pub(crate) async fn call_before_spawn(
        &mut self,
        mut context: BeforeSpawnContext,
    ) -> Result<BeforeSpawnContext, InitializationHookError>
    {
        for hook in &mut self.before_spawn {
            context = hook.call(context).await?;
        }
        Ok(context)
    }
}

pub trait AsyncHook<T>: Send + Sync + 'static {
    type Future: Future<Output = Result<T, InitializationHookError>> + Send;

    fn call(&mut self, context: T) -> Self::Future;

    fn boxed(self) -> BoxedAsyncHook<T>
    where
        Self: Sized + Send + Sync + 'static,
        Self::Future: 'static,
    {
        BoxedAsyncHook { inner: Box::new(self) }
    }
}

pub struct BoxedAsyncHook<T> {
    inner: Box<dyn BoxAsyncHook<T>>,
}

impl<T> BoxedAsyncHook<T> {
    async fn call(&mut self, context: T) -> Result<T, InitializationHookError> {
        self.inner.call(context).await
    }
}

pub trait BoxAsyncHook<T>: Send + Sync {
    fn call(&mut self, context: T) -> BoxFuture<'static, Result<T, InitializationHookError>>;
}

impl<T, H> BoxAsyncHook<T> for H
where
    H: AsyncHook<T>,
    H::Future: 'static,
{
    fn call(&mut self, context: T) -> BoxFuture<'static, Result<T, InitializationHookError>> {
        self.call(context).boxed()
    }
}

struct FnWrapper<F> {
    inner: Option<F>,
}

impl<F> FnWrapper<F> {
    fn new(f: F) -> Self {
        Self { inner: Some(f) }
    }
}

impl<T, F, Fut> AsyncHook<T> for FnWrapper<F>
where
    F: FnOnce(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, InitializationHookError>> + Send + 'static,
    T: Send + 'static,
{
    type Future = Fut;

    fn call(&mut self, arg: T) -> Self::Future {
        match self.inner.take() {
            Some(func) => (func)(arg),
            None => panic!("Hook called more than once"),
        }
    }
}

impl<T, F, Fut> AsyncHook<T> for F
where
    F: FnMut(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, InitializationHookError>> + Send + 'static,
    T: Send + 'static,
{
    type Future = Fut;

    fn call(&mut self, arg: T) -> Self::Future {
        (self)(arg)
    }
}

pub struct BeforeSpawnContext {
    pub(crate) peers: Vec<Peer>,
    pub(crate) protocols: ProtocolExtensions,
    node_identity: Arc<NodeIdentity>,
    outbound_requester: OutboundMessageRequester,
}

impl BeforeSpawnContext {
    pub(crate) fn new(node_identity: Arc<NodeIdentity>, outbound_requester: OutboundMessageRequester) -> Self {
        Self {
            peers: Vec::new(),
            protocols: ProtocolExtensions::new(),
            node_identity,
            outbound_requester,
        }
    }

    pub fn node_identity(&self) -> &NodeIdentity {
        &self.node_identity
    }

    pub fn add_peers<T: IntoIterator<Item = Peer>>(&mut self, peers: T) -> &mut Self {
        self.peers.extend(peers);
        self
    }

    pub fn outbound_requester(&self) -> &OutboundMessageRequester {
        &self.outbound_requester
    }

    pub fn add_peer(&mut self, peer: Peer) -> &mut Self {
        self.peers.push(peer);
        self
    }

    pub fn add_protocol_extension<T: ProtocolExtension + Send + 'static>(&mut self, extension: T) -> &mut Self {
        self.protocols.add(extension);
        self
    }
}
