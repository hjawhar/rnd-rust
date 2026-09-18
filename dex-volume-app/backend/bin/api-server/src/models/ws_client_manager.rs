use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

struct WsClient {
    user_id: i32,
    project_ids: HashSet<i32>,
    tx: mpsc::Sender<Arc<String>>,
}

/// Internal state protected by a single RwLock.
/// Keeping the primary map and secondary indexes together ensures they
/// never diverge — every mutation touches all three atomically.
struct ClientStore {
    clients: HashMap<u64, WsClient>,
    /// Secondary index: project_id → set of client_ids subscribed to that project.
    project_index: HashMap<i32, HashSet<u64>>,
    /// Secondary index: user_id → set of client_ids belonging to that user.
    user_index: HashMap<i32, HashSet<u64>>,
}

impl ClientStore {
    fn new() -> Self {
        Self {
            clients: HashMap::new(),
            project_index: HashMap::new(),
            user_index: HashMap::new(),
        }
    }

    /// Remove a client and clean up all index entries.
    fn remove(&mut self, client_id: u64) {
        if let Some(client) = self.clients.remove(&client_id) {
            for &pid in &client.project_ids {
                if let Some(set) = self.project_index.get_mut(&pid) {
                    set.remove(&client_id);
                    if set.is_empty() {
                        self.project_index.remove(&pid);
                    }
                }
            }
            if let Some(set) = self.user_index.get_mut(&client.user_id) {
                set.remove(&client_id);
                if set.is_empty() {
                    self.user_index.remove(&client.user_id);
                }
            }
        }
    }
}

pub struct WsClientManager {
    store: RwLock<ClientStore>,
    next_id: AtomicU64,
}

impl Default for WsClientManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WsClientManager {
    pub fn new() -> Self {
        Self {
            store: RwLock::new(ClientStore::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a new WebSocket client. Returns (client_id, receiver).
    pub async fn register(
        &self,
        user_id: i32,
        project_ids: Vec<i32>,
    ) -> (u64, mpsc::Receiver<Arc<String>>) {
        let (tx, rx) = mpsc::channel(1000);
        let client_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let project_set: HashSet<i32> = project_ids.into_iter().collect();

        let mut store = self.store.write().await;
        // Build indexes
        for &pid in &project_set {
            store.project_index.entry(pid).or_default().insert(client_id);
        }
        store.user_index.entry(user_id).or_default().insert(client_id);

        store.clients.insert(client_id, WsClient {
            user_id,
            project_ids: project_set,
            tx,
        });
        (client_id, rx)
    }

    /// Unregister a client by ID.
    pub async fn unregister(&self, client_id: u64) {
        self.store.write().await.remove(client_id);
    }

    /// Broadcast a pre-serialized JSON message to all connected clients.
    pub async fn broadcast_all(&self, json: Arc<String>) {
        let mut stale = Vec::new();
        {
            let store = self.store.read().await;
            for (&id, client) in store.clients.iter() {
                if client.tx.try_send(json.clone()).is_err() {
                    stale.push(id);
                }
            }
        }
        if !stale.is_empty() {
            let mut store = self.store.write().await;
            for id in stale {
                store.remove(id);
                tracing::debug!("[WS] Removed stale client {id}");
            }
        }
    }

    /// Send a pre-serialized JSON message only to clients authorized for a project.
    /// O(K) where K = number of clients subscribed to this project.
    pub async fn send_to_project(&self, project_id: i32, json: Arc<String>) {
        let mut stale = Vec::new();
        {
            let store = self.store.read().await;
            if let Some(client_ids) = store.project_index.get(&project_id) {
                for &cid in client_ids {
                    if let Some(client) = store.clients.get(&cid)
                        && client.tx.try_send(json.clone()).is_err() {
                            stale.push(cid);
                        }
                }
            }
        }
        if !stale.is_empty() {
            let mut store = self.store.write().await;
            for id in stale {
                store.remove(id);
                tracing::debug!("[WS] Removed stale client {id}");
            }
        }
    }

    /// Send a pre-serialized JSON message to all connections of a specific user.
    /// O(K) where K = number of connections for this user.
    pub async fn send_to_user(&self, user_id: i32, json: Arc<String>) {
        let mut stale = Vec::new();
        {
            let store = self.store.read().await;
            if let Some(client_ids) = store.user_index.get(&user_id) {
                for &cid in client_ids {
                    if let Some(client) = store.clients.get(&cid)
                        && client.tx.try_send(json.clone()).is_err() {
                            stale.push(cid);
                        }
                }
            }
        }
        if !stale.is_empty() {
            let mut store = self.store.write().await;
            for id in stale {
                store.remove(id);
                tracing::debug!("[WS] Removed stale client {id}");
            }
        }
    }

    /// Add a project_id to all connections of a user (e.g. after grant_project_access).
    pub async fn add_project_for_user(&self, user_id: i32, project_id: i32) {
        let mut store = self.store.write().await;
        let client_ids: Vec<u64> = store.user_index
            .get(&user_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for cid in client_ids {
            if let Some(client) = store.clients.get_mut(&cid)
                && client.project_ids.insert(project_id) {
                    store.project_index.entry(project_id).or_default().insert(cid);
                }
        }
    }

    /// Remove a project_id from all connections of a user (e.g. after revoke_project_access).
    pub async fn remove_project_for_user(&self, user_id: i32, project_id: i32) {
        let mut store = self.store.write().await;
        let client_ids: Vec<u64> = store.user_index
            .get(&user_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for cid in client_ids {
            if let Some(client) = store.clients.get_mut(&cid)
                && client.project_ids.remove(&project_id)
                    && let Some(set) = store.project_index.get_mut(&project_id) {
                        set.remove(&cid);
                        if set.is_empty() {
                            store.project_index.remove(&project_id);
                        }
                    }
        }
    }

    /// Remove a project_id from all clients (e.g. after project deletion).
    pub async fn remove_project_for_all(&self, project_id: i32) {
        let mut store = self.store.write().await;
        if let Some(client_ids) = store.project_index.remove(&project_id) {
            for cid in client_ids {
                if let Some(client) = store.clients.get_mut(&cid) {
                    client.project_ids.remove(&project_id);
                }
            }
        }
    }

    /// Return the number of connected clients.
    pub async fn client_count(&self) -> usize {
        self.store.read().await.clients.len()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn msg(s: &str) -> Arc<String> {
        Arc::new(s.to_string())
    }

    #[tokio::test]
    async fn register_and_unregister() {
        let mgr = WsClientManager::new();
        assert_eq!(mgr.client_count().await, 0);

        let (id1, _rx1) = mgr.register(1, vec![10, 20]).await;
        let (id2, _rx2) = mgr.register(2, vec![30]).await;
        assert_eq!(mgr.client_count().await, 2);
        assert_ne!(id1, id2, "client IDs must be unique");

        mgr.unregister(id1).await;
        assert_eq!(mgr.client_count().await, 1);

        mgr.unregister(id2).await;
        assert_eq!(mgr.client_count().await, 0);
    }

    #[tokio::test]
    async fn unregister_nonexistent_is_noop() {
        let mgr = WsClientManager::new();
        mgr.unregister(999).await;
        assert_eq!(mgr.client_count().await, 0);
    }

    #[tokio::test]
    async fn broadcast_all_delivers_to_every_client() {
        let mgr = WsClientManager::new();
        let (_id1, mut rx1) = mgr.register(1, vec![10]).await;
        let (_id2, mut rx2) = mgr.register(2, vec![20]).await;

        mgr.broadcast_all(msg("hello")).await;

        let m1 = rx1.try_recv().expect("client 1 must receive");
        let m2 = rx2.try_recv().expect("client 2 must receive");
        assert_eq!(*m1, "hello");
        assert_eq!(*m2, "hello");
    }

    #[tokio::test]
    async fn send_to_project_routes_correctly() {
        let mgr = WsClientManager::new();
        let (_id1, mut rx1) = mgr.register(1, vec![10, 20]).await;
        let (_id2, mut rx2) = mgr.register(2, vec![20, 30]).await;
        let (_id3, mut rx3) = mgr.register(3, vec![30]).await;

        mgr.send_to_project(20, msg("proj20")).await;

        // Clients 1 and 2 have project 20
        let m1 = rx1.try_recv().expect("client 1 must receive (has project 20)");
        let m2 = rx2.try_recv().expect("client 2 must receive (has project 20)");
        assert_eq!(*m1, "proj20");
        assert_eq!(*m2, "proj20");

        // Client 3 does NOT have project 20
        assert!(rx3.try_recv().is_err(), "client 3 must not receive (no project 20)");
    }

    #[tokio::test]
    async fn send_to_user_routes_correctly() {
        let mgr = WsClientManager::new();
        // Two connections for user 1, one for user 2
        let (_id1a, mut rx1a) = mgr.register(1, vec![10]).await;
        let (_id1b, mut rx1b) = mgr.register(1, vec![20]).await;
        let (_id2, mut rx2) = mgr.register(2, vec![10]).await;

        mgr.send_to_user(1, msg("user1")).await;

        rx1a.try_recv().expect("user 1 conn A must receive");
        rx1b.try_recv().expect("user 1 conn B must receive");
        assert!(rx2.try_recv().is_err(), "user 2 must not receive");
    }

    #[tokio::test]
    async fn stale_client_evicted_on_broadcast() {
        let mgr = WsClientManager::new();
        let (id1, rx1) = mgr.register(1, vec![10]).await;
        let (_id2, mut rx2) = mgr.register(2, vec![10]).await;

        // Drop rx1 — makes the sender half fail on try_send
        drop(rx1);

        mgr.broadcast_all(msg("test")).await;

        // Client 1 should be evicted, client 2 still present
        assert_eq!(mgr.client_count().await, 1);
        rx2.try_recv().expect("client 2 must still receive");
    }

    #[tokio::test]
    async fn stale_client_evicted_on_send_to_project() {
        let mgr = WsClientManager::new();
        let (_id1, rx1) = mgr.register(1, vec![10]).await;
        let (_id2, mut rx2) = mgr.register(2, vec![10]).await;

        drop(rx1);

        mgr.send_to_project(10, msg("test")).await;

        assert_eq!(mgr.client_count().await, 1);
        rx2.try_recv().expect("client 2 must still receive");
    }

    #[tokio::test]
    async fn add_project_for_user_enables_routing() {
        let mgr = WsClientManager::new();
        let (_id1, mut rx1) = mgr.register(1, vec![10]).await;

        // Initially, user 1 does NOT have project 20
        mgr.send_to_project(20, msg("before")).await;
        assert!(rx1.try_recv().is_err(), "must not receive before add");

        // Add project 20
        mgr.add_project_for_user(1, 20).await;

        mgr.send_to_project(20, msg("after")).await;
        rx1.try_recv().expect("must receive after add");
    }

    #[tokio::test]
    async fn add_project_for_user_is_idempotent() {
        let mgr = WsClientManager::new();
        let (id, _rx) = mgr.register(1, vec![10]).await;

        mgr.add_project_for_user(1, 10).await; // already has 10
        mgr.add_project_for_user(1, 10).await; // duplicate

        // Should still have exactly [10], not [10, 10, 10]
        let store = mgr.store.read().await;
        let client = store.clients.get(&id).unwrap();
        assert_eq!(client.project_ids.len(), 1);
    }

    #[tokio::test]
    async fn remove_project_for_user_disables_routing() {
        let mgr = WsClientManager::new();
        let (_id1, mut rx1) = mgr.register(1, vec![10, 20]).await;

        mgr.remove_project_for_user(1, 20).await;

        mgr.send_to_project(20, msg("gone")).await;
        assert!(rx1.try_recv().is_err(), "must not receive after remove");

        // Still receives for project 10
        mgr.send_to_project(10, msg("still")).await;
        rx1.try_recv().expect("must still receive for project 10");
    }

    #[tokio::test]
    async fn remove_project_for_all() {
        let mgr = WsClientManager::new();
        let (_id1, mut rx1) = mgr.register(1, vec![10, 20]).await;
        let (_id2, mut rx2) = mgr.register(2, vec![20, 30]).await;

        mgr.remove_project_for_all(20).await;

        mgr.send_to_project(20, msg("deleted")).await;
        assert!(rx1.try_recv().is_err(), "client 1 must not receive");
        assert!(rx2.try_recv().is_err(), "client 2 must not receive");

        // Other projects unaffected
        mgr.send_to_project(10, msg("ok")).await;
        rx1.try_recv().expect("client 1 must still receive for project 10");
        mgr.send_to_project(30, msg("ok")).await;
        rx2.try_recv().expect("client 2 must still receive for project 30");
    }

    #[tokio::test]
    async fn send_to_nonexistent_project_is_noop() {
        let mgr = WsClientManager::new();
        let (_id, mut rx) = mgr.register(1, vec![10]).await;

        mgr.send_to_project(999, msg("ghost")).await;
        assert!(rx.try_recv().is_err(), "must not receive for unsubscribed project");
    }
}