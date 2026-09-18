use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use super::{
    buy::BuyRequest,
    latency::Latency,
    mpsc::{GeyserTracker, MpscLogs, MpscTask},
    tasks::TaskWallet,
};

#[derive(Clone)]
pub struct AppState {
    pub server_name: String,
    pub axsys_external_api_key: String,
    pub axsys_ws_endpoint: String,
    pub axsys_api_key: String,
    pub axsys_api_key_normal: String,
    pub ocr_api_endpoint: String,
    pub rpc_endpoint: String,
    pub geyser_endpoint: String,
    pub geyser_token: String,
    pub jupiter_endpoint: String,
    pub jupiter_api_key: String,
    pub nextblock_endpoint: String,
    pub nextblock_api_key: String,
    pub jito_api_endpoint: String,
    pub jito_api_key: String,

    pub tx_task: tokio::sync::mpsc::Sender<MpscTask>,
    pub tx_logs: tokio::sync::mpsc::Sender<MpscLogs>,
    pub ws_clients: Arc<RwLock<HashMap<String, Latency>>>,
    pub tasks: Arc<RwLock<Vec<TaskWallet>>>,

    pub spam_tasks: Arc<RwLock<Vec<BuyRequest>>>,
    pub spam_tasks_available: Arc<RwLock<HashMap<String, bool>>>,
    pub tasks_can_retry: Arc<RwLock<HashMap<String, bool>>>,
    pub tweets_detected: Arc<RwLock<HashMap<String, bool>>>,
    pub tweets_bought: Arc<RwLock<HashMap<String, bool>>>,
    pub tweets_images: Arc<RwLock<HashMap<String, bool>>>,
    pub retries: Arc<RwLock<HashMap<String, i32>>>,
    pub tx_geyser_tracker: tokio::sync::mpsc::Sender<GeyserTracker>,
}

impl AppState {
    // Tasks
    pub async fn set_tasks(&self, new_tasks: &Vec<TaskWallet>) {
        let mut tasks = self.tasks.write().await;
        *tasks = new_tasks.clone();
        drop(tasks);
    }

    pub async fn add_task(&self, new_task: &TaskWallet) {
        let mut tasks = self.tasks.write().await;
        let mut current_tasks = tasks.clone();
        current_tasks.push(new_task.clone());
        *tasks = current_tasks;
        drop(tasks);
    }

    pub async fn get_task(&self, task_id: i32) -> Option<TaskWallet> {
        let tasks_guard = self.tasks.read().await;
        let tasks = tasks_guard.clone();
        drop(tasks_guard);
        let found = tasks.iter().find(|x| x.id == task_id);
        return found.cloned();
    }

    pub async fn get_tasks(&self) -> Vec<TaskWallet> {
        let tasks_guard = self.tasks.read().await;
        let tasks = tasks_guard.clone();
        drop(tasks_guard);
        return tasks.clone();
    }

    pub async fn get_tasks_with_pub_key(&self, pub_key: String) -> Vec<TaskWallet> {
        let tasks_guard = self.tasks.read().await;
        let tasks = tasks_guard.clone();
        drop(tasks_guard);
        let tasks: Vec<_> = tasks
            .clone()
            .iter()
            .filter(|x| x.public_key.is_some() && x.public_key.clone().unwrap().eq(&pub_key))
            .map(|x| x.clone())
            .collect();
        return tasks.clone();
    }

    pub async fn update_task(&self, updated_task: &TaskWallet) {
        let mut tasks = self.tasks.write().await;
        let mut current_tasks = tasks.clone();
        let index = current_tasks.iter().position(|x| x.id == updated_task.id);
        if let Some(i) = index {
            current_tasks[i] = updated_task.clone();
            *tasks = current_tasks;
        }
        drop(tasks);
    }

    pub async fn remove_task(&self, task_id: &i32) {
        let mut tasks = self.tasks.write().await;
        let mut current_tasks = tasks.clone();
        let index = current_tasks.iter().position(|x| x.id == *task_id);
        if let Some(i) = index {
            current_tasks.remove(i);
            *tasks = current_tasks;
        }
        drop(tasks);
    }
    // Tweets detected

    pub async fn add_tweet_detected(&self, tweet_id: String) -> String {
        let mut tweets_map = self.tweets_detected.write().await;
        let mut current_tweets_map = tweets_map.clone();
        current_tweets_map.insert(tweet_id.clone(), true);
        *tweets_map = current_tweets_map;
        drop(tweets_map);
        tweet_id
    }

    pub async fn get_tweet_detected(&self, id: String) -> Option<bool> {
        let tweets_map_guard = self.tweets_detected.read().await;
        let tweets_map = tweets_map_guard.clone();
        drop(tweets_map_guard);
        let found = tweets_map.get(&id);
        return found.cloned();
    }

    // Tweets images

    pub async fn add_tweet_image(&self, tweet_image: String) {
        let mut tweets_images = self.tweets_images.write().await;
        let mut current_tweets_images = tweets_images.clone();
        current_tweets_images.insert(tweet_image, true);
        *tweets_images = current_tweets_images;
        drop(tweets_images);
    }

    pub async fn get_tweet_image(&self, id: String) -> Option<bool> {
        let tweets_images_guard = self.tweets_images.read().await;
        let tweets_images = tweets_images_guard.clone();
        drop(tweets_images_guard);
        let found = tweets_images.get(&id);
        return found.cloned();
    }

    // Tweets bought

    pub async fn add_tweet_bought(&self, tweet_id: String) -> String {
        let mut tweets_map = self.tweets_bought.write().await;
        let mut current_tweets_map = tweets_map.clone();
        current_tweets_map.insert(tweet_id.clone(), true);
        *tweets_map = current_tweets_map;
        drop(tweets_map);
        tweet_id
    }

    pub async fn get_tweet_bought(&self, id: String) -> Option<bool> {
        let tweets_map_guard = self.tweets_bought.read().await;
        let tweets_map = tweets_map_guard.clone();
        drop(tweets_map_guard);
        let found = tweets_map.get(&id);
        return found.cloned();
    }

    // Spam tasks
    pub async fn add_spam_task(&self, new_task: &BuyRequest) {
        let mut spam_tasks = self.spam_tasks.write().await;
        let mut current_spam_tasks = spam_tasks.clone();
        current_spam_tasks.push(new_task.clone());
        *spam_tasks = current_spam_tasks;
        drop(spam_tasks);
    }

    pub async fn get_spam_tasks(&self) -> Vec<BuyRequest> {
        let spam_tasks_guard = self.spam_tasks.read().await;
        let spam_tasks = spam_tasks_guard.clone();
        drop(spam_tasks_guard);
        return spam_tasks.clone();
    }

    pub async fn remove_spam_tasks_by_uuid(&self, uuid: String) {
        let mut spam_tasks = self.spam_tasks.write().await;
        let new_spam_tasks: Vec<_> = spam_tasks
            .clone()
            .iter()
            .filter(|x| x.uuid != uuid)
            .map(|x| x.clone())
            .collect();

        *spam_tasks = new_spam_tasks;
        drop(spam_tasks);
    }

    // Retries
    pub async fn get_all_retries(&self) -> HashMap<String, i32> {
        let retries_guard = self.retries.read().await;
        let retries = retries_guard.clone();
        drop(retries_guard);
        return retries.clone();
    }

    pub async fn get_retries(&self, id: String) -> Option<i32> {
        let retries_guard = self.retries.read().await;
        let retries = retries_guard.clone();
        drop(retries_guard);
        let found = retries.get(&id);
        return found.copied();
    }

    pub async fn add_retries(&self, id: String, new_retries: i32) -> i32 {
        let mut retries_guard = self.retries.write().await;
        let mut retries = retries_guard.clone();
        retries.insert(id, new_retries);
        *retries_guard = retries;
        drop(retries_guard);
        new_retries
    }

    // Websocket Clients
    pub async fn get_ws_clients(&self) -> HashMap<String, Latency> {
        let ws_clients_guard = self.ws_clients.read().await;
        let ws_clients = ws_clients_guard.clone();
        drop(ws_clients_guard);
        return ws_clients;
    }

    pub async fn insert_ws_client(
        &self,
        id: String,
        ping: Option<i128>,
        pong: Option<i128>,
    ) -> Latency {
        let mut ws_clients_guard = self.ws_clients.write().await;
        let new_latency: Latency;
        let mut ws_clients = ws_clients_guard.clone();
        if let Some(found) = ws_clients.get(&id) {
            new_latency = Latency {
                ping: ping.unwrap_or(found.ping),
                pong: pong.unwrap_or(found.pong),
            }
        } else {
            new_latency = Latency {
                ping: ping.unwrap_or(0),
                pong: pong.unwrap_or(0),
            }
        }
        ws_clients.insert(id, new_latency.clone());
        *ws_clients_guard = ws_clients;
        drop(ws_clients_guard);
        new_latency.clone()
    }

    pub async fn remove_ws_client(&self, id: String) {
        let mut ws_clients_guard = self.ws_clients.write().await;
        let mut ws_clients = ws_clients_guard.clone();
        ws_clients.remove(&id);
        *ws_clients_guard = ws_clients;
        drop(ws_clients_guard);
    }
}
