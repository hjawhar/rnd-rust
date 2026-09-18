use std::sync::Arc;

use uuid::Uuid;

use crate::{
    api::{ocr::request_ocr_data, pushover::notify_all_users, solana::full_buy},
    models::{
        axsys::{TweetEvent, TwitterMiniTweet},
        buy::BuyRequest,
        mpsc::{LogsType, MpscLogs, TaskLogs, TweetLogs, TxStatus},
        state::AppState,
        tasks::TaskWallet,
    },
    utils::helpers::{
        adjust_tweet_img, analyze_ocr, analyze_ocr_concatenated, analyze_text, big_int_to_f64,
        bruteforce_text, detect_bs58, get_current_time_ms,
    },
};

async fn inner_buy(
    state: Arc<AppState>,
    twitter_mini_tweet: TwitterMiniTweet,
    tasks: Vec<TaskWallet>,
    mint_address: Option<String>,
) {
    let tweet_id = twitter_mini_tweet.id.clone();

    if let Some(_) = state.get_tweet_bought(twitter_mini_tweet.id.clone()).await {
        tracing::info!("Already bought for tweet {}", twitter_mini_tweet.id.clone());
        return;
    } else {
        if state.get_retries(tweet_id.clone()).await.is_none() {
            state.add_retries(tweet_id.clone(), 1).await;
        }

        let mut search_text = twitter_mini_tweet.clone().body.text;
        if let Some(subtweet) = twitter_mini_tweet.subtweet.clone() {
            search_text = format!("{} {}", search_text, subtweet.body.text);
        }

        for task in tasks {
            let strats: Vec<String> = task
                .twitter_strategy
                .clone()
                .unwrap()
                .split(",")
                .map(|x| x.to_string())
                .collect();

            let found_strategy = strats
                .iter()
                .find(|x| x.contains(&twitter_mini_tweet.r#type));

            if let Some(found_stategy) = found_strategy {
                let mut final_mint_address: Option<String> = mint_address.clone();
                if let Some(twitter_token_override) = task.twitter_token_override {
                    final_mint_address = Some(twitter_token_override);
                }

                if found_stategy == "RETWEET"
                    || found_stategy == "QUOTE"
                    || found_stategy == "REPLY"
                {
                    if let Some(twitter_handle_checker) = task.twitter_handle_checker {
                        if let Some(ref reply) = twitter_mini_tweet.reply {
                            let found = reply.handle == twitter_handle_checker;
                            // && final_mint_address.is_some()
                            // && twitter_mini_tweet
                            //     .body
                            //     .text
                            //     .contains(&final_mint_address.clone().unwrap());
                            if !found {
                                continue;
                            }
                        }
                    }
                }

                if let Some(keywords) = task.words {
                    let keywords: Vec<&str> = keywords.split(",").collect();
                    if keywords.len() > 0 {
                        let found = keywords
                            .iter()
                            .find(|keyword| search_text.contains(*keyword));
                        if let None = found {
                            continue;
                        }
                    }
                }

                if let Some(final_mint_address) = final_mint_address {
                    let buy_req = BuyRequest {
                        retry_uuid: Uuid::new_v4().to_string(),
                        uuid: tweet_id.clone(),
                        private_key: task.private_key.clone().unwrap(),
                        nonce_account_address: task.nonce_account_address.clone().unwrap(),
                        mint: final_mint_address.clone(),
                        value: big_int_to_f64(task.value.clone().unwrap()),
                        tip: big_int_to_f64(task.tip.clone().unwrap()),
                        slippage: task.slippage,
                        tries: task.tries,
                        frontrunning_protection: task.frontrunning_protection,
                        enable_alerts: task.enable_alerts,
                        servers: task.servers.clone(),
                        block_leaders: task.block_leaders.clone(),
                        selected_pool: task.selected_pool.clone(),
                    };

                    tracing::info!(
                        "Adding tweet bought flag for {}",
                        twitter_mini_tweet.id.clone()
                    );
                    if let None = state.get_tweet_bought(twitter_mini_tweet.id.clone()).await {
                        state.add_tweet_bought(twitter_mini_tweet.id.clone()).await;
                        full_buy(state.clone(), buy_req).await;
                    }
                }
            }
        }
    }
}

pub async fn handle_tweet(state: Arc<AppState>, tweet: TweetEvent, slot: u64) {
    let twitter_mini_tweet = tweet.clone().tweet;
    let mut search_text = twitter_mini_tweet.clone().body.text;

    if let Some(subtweet) = twitter_mini_tweet.subtweet.clone() {
        search_text = format!("{} {}", search_text, subtweet.body.text);
    }

    for url in &twitter_mini_tweet.body.urls {
        search_text = format!("{} {}", search_text, url.url);
    }

    let mut found = detect_bs58(search_text.clone());
    tracing::info!("Text: {:#?}", search_text);
    tracing::info!("Found CA in text: {:#?}", found);

    if let None = state
        .get_tweet_detected(twitter_mini_tweet.id.clone())
        .await
    {
        state
            .add_tweet_detected(twitter_mini_tweet.id.clone())
            .await;
        let _ = state
            .tx_logs
            .send(MpscLogs::Tweet(TweetLogs {
                slot,
                tweet: twitter_mini_tweet.clone(),
                timestamp: get_current_time_ms(),
            }))
            .await;
    }

    if let Some(_) = state.get_tweet_bought(twitter_mini_tweet.id.clone()).await {
        tracing::info!("Already bought for tweet {}", twitter_mini_tweet.id.clone());
        return;
    }

    let tasks = state.get_tasks().await;
    let tweet_handle = twitter_mini_tweet.author.handle.clone();
    let tasks: Vec<_> = tasks
        .iter()
        .filter(|task| {
            task.twitter_handle.is_some()
                && task.twitter_handle.clone().unwrap().eq(&tweet_handle)
                && task.value.is_some()
                && task.tip.is_some()
                && task.private_key.is_some()
                && task.twitter_strategy.is_some()
                && task.nonce_account_address.is_some()
        })
        .map(|x| x.clone())
        .collect();

    inner_buy(
        state.clone(),
        twitter_mini_tweet.clone(),
        tasks.clone(),
        None,
    )
    .await;

    if let Some(found) = found {
        inner_buy(
            state.clone(),
            twitter_mini_tweet.clone(),
            tasks.clone(),
            Some(found.clone()),
        )
        .await;
    } else {
        let state = state.clone();
        let twitter_mini_tweet = twitter_mini_tweet.clone();
        let cloned_tasks = tasks.clone();
        tokio::task::spawn(async move {
            let bruteforced_results = bruteforce_text(&search_text).await;
            if let Ok(bruteforced_results) = bruteforced_results {
                if let Some(found_bruteforced) = bruteforced_results {
                    inner_buy(
                        state.clone(),
                        twitter_mini_tweet.clone(),
                        cloned_tasks.clone(),
                        Some(found_bruteforced.clone()),
                    )
                    .await;
                }
            }
        });
    }

    if twitter_mini_tweet.media.images.len() > 0 {
        handle_images(state.clone(), twitter_mini_tweet.clone(), tasks.clone()).await;
    }
}

pub async fn handle_images(
    state: Arc<AppState>,
    twitter_mini_tweet: TwitterMiniTweet,
    tasks: Vec<TaskWallet>,
) {
    tracing::info!(
        "Retrieving tweet images: {:#?}",
        twitter_mini_tweet.media.images
    );
    for current_img in twitter_mini_tweet.media.images.clone() {
        let img = adjust_tweet_img(current_img);
        // let img = img.clone();
        let state = state.clone();
        let tmt = twitter_mini_tweet.clone();
        if let None = state.get_tweet_image(img.clone()).await {
            state.add_tweet_image(img.clone()).await;
            let tasks = tasks.clone();
            tokio::task::spawn(async move {
                tracing::info!("Spawning image proces task for {img}");
                // let mut tries = 0;
                // loop {
                //     tries += 1;
                //     if tries == 10 {
                //         break;
                //     }
                tracing::info!("Trying to fetch image data for {img}");
                process_image_and_buy(state.clone(), tmt.clone(), img.clone(), tasks).await;
                tracing::info!("Done fetching image data for {img}");
                // tokio::time::sleep(Duration::from_millis(100)).await;
                // }
            });
        }
    }
}

async fn process_image_and_buy(
    state: Arc<AppState>,
    twitter_mini_tweet: TwitterMiniTweet,
    img: String,
    tasks: Vec<TaskWallet>,
) {
    let response_ocr = request_ocr_data(state.ocr_api_endpoint.clone(), img.clone()).await;
    if let Ok(response_ocr) = response_ocr {
        tracing::info!("OCR result for {img}: {response_ocr:#?}");
        let analyze_ocr = analyze_ocr(response_ocr.clone());
        tracing::info!("OCR analysis for {img}: {analyze_ocr:#?}");
        let analyzed_text = analyze_text(response_ocr.clone());
        let analyzed_text_concat: Vec<String> =
            analyzed_text.iter().map(|x| x.text.clone()).collect();
        let final_analyzed_text = analyzed_text_concat.join("\n");
        let _ = state
            .tx_logs
            .send(MpscLogs::Tx(TaskLogs {
                logs_type: LogsType::IMAGE,
                tx_hash: None,
                token: None,
                bundle_hash: None,
                sender: None,
                text: format!(
                    "First check - Text detected: {final_analyzed_text} - {}",
                    if let Some(ocr_res) = &analyze_ocr {
                        format!("Token CA found: {}", ocr_res.text)
                    } else {
                        format!("No token CA found.")
                    }
                ),
                confirmed: false,
                status: TxStatus::SENT,
                timestamp: get_current_time_ms(),
            }))
            .await;

        if let Some(analysis) = analyze_ocr {
            if let None = state.get_tweet_bought(twitter_mini_tweet.id.clone()).await {
                let _ = state
                    .tx_logs
                    .send(MpscLogs::Tx(TaskLogs {
                        logs_type: LogsType::IMAGE,
                        tx_hash: None,
                        token: None,
                        bundle_hash: None,
                        sender: None,
                        text: format!(
                            "Trying to buy {} using OCR with confidence score of {}",
                            analysis.text.clone(),
                            (analysis.confidence * 100.0).round() / 100.0
                        ),
                        confirmed: false,
                        status: TxStatus::SENT,
                        timestamp: get_current_time_ms(),
                    }))
                    .await;
            } else {
                tracing::info!(
                    "OCR - Already bought for tweet {}",
                    twitter_mini_tweet.id.clone()
                );
                return;
            }

            inner_buy(
                state.clone(),
                twitter_mini_tweet.clone(),
                tasks.clone(),
                Some(analysis.text.clone()),
            )
            .await;
        } else {
            let ocr_concatenated_res = analyze_ocr_concatenated(response_ocr);
            if let Some(ocr_concatenated_res) = ocr_concatenated_res {
                let bruteforce_res = bruteforce_text(&ocr_concatenated_res).await;
                if let Ok(bruteforce_res) = bruteforce_res {
                    let _ = state
                        .tx_logs
                        .send(MpscLogs::Tx(TaskLogs {
                            logs_type: LogsType::IMAGE,
                            tx_hash: None,
                            token: None,
                            bundle_hash: None,
                            sender: None,
                            text: format!(
                                "Second check - Text detected: {final_analyzed_text} - {}",
                                if let Some(ocr_res) = &bruteforce_res {
                                    format!("Token CA found: {}", ocr_res)
                                } else {
                                    format!("No token CA found.")
                                }
                            ),
                            confirmed: false,
                            status: TxStatus::SENT,
                            timestamp: get_current_time_ms(),
                        }))
                        .await;

                    if let Some(mint_address_detected) = bruteforce_res {
                        tracing::info!(
                            "Detected address from secondary OCR check: {mint_address_detected:#?}"
                        );
                        if let None = state.get_tweet_bought(twitter_mini_tweet.id.clone()).await {
                            let _ = state
                                .tx_logs
                                .send(MpscLogs::Tx(TaskLogs {
                                    logs_type: LogsType::IMAGE,
                                    tx_hash: None,
                                    token: None,
                                    bundle_hash: None,
                                    sender: None,
                                    text: format!(
                                        "[2] Trying to buy {} using OCR with confidence score of {}",
                                        mint_address_detected.clone(),
                                        100.0
                                    ),
                                    confirmed: false,
                                    status: TxStatus::SENT,
                                    timestamp: get_current_time_ms(),
                                }))
                                .await;
                        } else {
                            tracing::info!(
                                "OCR - Already bought for tweet {}",
                                twitter_mini_tweet.id.clone()
                            );
                            return;
                        }

                        inner_buy(
                            state.clone(),
                            twitter_mini_tweet.clone(),
                            tasks.clone(),
                            Some(mint_address_detected.clone()),
                        )
                        .await;
                    }
                }
            }
        }
    } else {
        tracing::info!("Error fetching ocr data for {img}");
    }
}
