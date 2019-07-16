pub mod application_agent;
pub mod bundlepack;
pub mod helpers;
pub mod processing;
pub mod store;

use crate::cla::ConvergencyLayerAgent;
use crate::routing::RoutingAgent;
use crate::CONFIG;
use crate::PEERS;
use crate::STATS;
use crate::STORE;
use application_agent::ApplicationAgent;
use bp7::EndpointID;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum PeerType {
    Static,
    Dynamic,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DtnPeer {
    pub eid: EndpointID,
    pub addr: IpAddr,
    pub con_type: PeerType,
    pub cla_list: Vec<(String, Option<u16>)>,
    pub last_contact: u64,
}

impl DtnPeer {
    pub fn new(
        eid: EndpointID,
        addr: IpAddr,
        con_type: PeerType,
        cla_list: Vec<(String, Option<u16>)>,
    ) -> DtnPeer {
        DtnPeer {
            eid,
            addr,
            con_type,
            cla_list,
            last_contact: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }
    /// Example
    ///
    /// ```
    /// use std::{thread, time};
    /// use dtn7::core::*;
    /// use dtn7::CONFIG;
    ///
    /// let mut peer = helpers::rnd_peer();
    /// let original_time = peer.last_contact;
    /// thread::sleep(time::Duration::from_secs(1));
    /// peer.touch();
    /// assert!(original_time < peer.last_contact);
    /// ```
    pub fn touch(&mut self) {
        self.last_contact = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    /// Example
    ///
    /// ```
    /// use std::{thread, time};
    /// use dtn7::core::*;
    /// use dtn7::CONFIG;
    ///
    /// CONFIG.lock().unwrap().peer_timeout = 1;
    /// let mut peer = helpers::rnd_peer();
    /// assert_eq!(peer.still_valid(), true);
    ///
    /// thread::sleep(time::Duration::from_secs(2));
    /// assert_eq!(peer.still_valid(), false);
    /// ```

    pub fn still_valid(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_contact < CONFIG.lock().unwrap().peer_timeout
    }

    pub fn get_node_name(&self) -> String {
        self.eid.node_part().unwrap_or_default()
    }
    pub fn get_first_cla(&self) -> Option<crate::cla::ClaSender> {
        for c in self.cla_list.iter() {
            if crate::cla::convergency_layer_agents().contains(&c.0.as_str()) {
                let sender = crate::cla::ClaSender {
                    remote: self.addr,
                    port: c.1,
                    agent: c.0.clone(),
                };
                return Some(sender);
            }
        }
        None
    }
}
pub fn peers_get_for_node(eid: &EndpointID) -> Option<DtnPeer> {
    for (_, p) in PEERS.lock().unwrap().iter() {
        if p.get_node_name() == eid.node_part().unwrap_or_default() {
            return Some(p.clone());
        }
    }
    None
}
pub fn peers_cla_for_node(eid: &EndpointID) -> Option<crate::cla::ClaSender> {
    if let Some(peer) = peers_get_for_node(eid) {
        return peer.get_first_cla();
    }
    None
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DtnStatistics {
    pub incoming: u64,
    pub dups: u64,
    pub outgoing: u64,
    pub delivered: u64,
    pub broken: u64,
}

impl DtnStatistics {
    pub fn new() -> DtnStatistics {
        DtnStatistics {
            incoming: 0,
            dups: 0,
            outgoing: 0,
            delivered: 0,
            broken: 0,
        }
    }
}
#[derive(Debug)]
pub struct DtnCore {
    pub endpoints: Vec<Box<dyn ApplicationAgent + Send>>,
    pub cl_list: Vec<Box<dyn ConvergencyLayerAgent>>,
    pub routing_agent: Box<RoutingAgent>,
}

impl Default for DtnCore {
    fn default() -> Self {
        Self::new()
    }
}

impl DtnCore {
    pub fn new() -> DtnCore {
        DtnCore {
            endpoints: Vec::new(),
            cl_list: Vec::new(),
            //routing_agent: Box::new(crate::routing::flooding::FloodingRoutingAgent::new()),
            routing_agent: Box::new(crate::routing::epidemic::EpidemicRoutingAgent::new()),
        }
    }

    pub fn register_application_agent<T: 'static + ApplicationAgent + Send>(&mut self, aa: T) {
        info!("Registered new application agent for EID: {}", aa.eid());
        self.endpoints.push(Box::new(aa));
    }
    pub fn unregister_application_agent<T: 'static + ApplicationAgent>(&mut self, aa: T) {
        info!("Unregistered application agent for EID: {}", aa.eid());
        self.endpoints
            .iter()
            .position(|n| n.eid() == aa.eid())
            .map(|e| self.endpoints.remove(e));
    }
    pub fn eids(&self) -> Vec<String> {
        self.endpoints.iter().map(|e| e.eid().to_string()).collect()
    }
    pub fn bundles(&self) -> Vec<String> {
        STORE.lock().unwrap().bundles().iter().map(|e| e.id()).collect()
    }
    fn is_in_endpoints(&self, eid: &EndpointID) -> bool {
        for aa in self.endpoints.iter() {
            if eid == aa.eid() {
                return true;
            }
        }
        false
    }
    pub fn get_endpoint_mut(
        &mut self,
        eid: &EndpointID,
    ) -> Option<&mut Box<dyn ApplicationAgent + Send>> {
        for aa in self.endpoints.iter_mut() {
            if eid == aa.eid() {
                return Some(aa);
            }
        }
        None
    }
    pub fn get_endpoint(&self, eid: &EndpointID) -> Option<&Box<dyn ApplicationAgent + Send>> {
        for aa in self.endpoints.iter() {
            if eid == aa.eid() {
                return Some(aa);
            }
        }
        None
    }
}

/// Removes peers from global peer list that haven't been seen in a while.
pub fn process_peers() {
    PEERS
        .lock()
        .unwrap()
        .retain(|_k, v| v.con_type == PeerType::Static || v.still_valid());
}
