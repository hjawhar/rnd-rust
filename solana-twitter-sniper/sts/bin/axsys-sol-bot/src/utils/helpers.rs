use crate::models::axsys::TweetEvent;
use crate::models::ocr::OcrCondifence;
use bigdecimal::BigDecimal;
use bigdecimal::ToPrimitive;
use bs58;
use dotenv::dotenv;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::error::Error;
use std::time::Instant;

use std::{
    collections::HashSet,
    env,
    ops::Mul,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};
pub fn mul_f64_and_u64_to_u64(x: f64, y: u64) -> u64 {
    (x.mul(y as f64).floor())
        .to_string()
        .parse::<u64>()
        .unwrap()
}

pub fn big_int_to_f64(input: BigDecimal) -> f64 {
    input.to_f64().unwrap()
}

pub fn get_current_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

pub fn detect_bs58(input: String) -> Option<String> {
    let mut alphanumeric_chars = input;
    let empty = " ".chars().nth(0).unwrap();
    alphanumeric_chars = alphanumeric_chars
        .chars()
        .map(|x| {
            if x.is_alphanumeric() || x.to_string().as_str() == " " {
                x
            } else {
                empty
            }
        })
        .collect();
    let alphanumeric_chars: Vec<_> = alphanumeric_chars
        .split(" ")
        .map(|x| x.to_string())
        .collect();

    let found = alphanumeric_chars
        .iter()
        .find(|x| Pubkey::from_str(x).is_ok());

    if let Some(found) = found {
        return Some(found.clone());
    } else {
        return None;
    }
}

pub fn adjust_tweet_img(input: String) -> String {
    let text_split: Vec<_> = input.split("/").collect();
    let img_url = text_split[text_split.len() - 1];
    let img_data: Vec<_> = img_url.split(".").collect();
    let img_id = img_data[0];
    let img_format = img_data[1];
    return format!("https://pbs.twimg.com/media/{img_id}?format={img_format}&name=small");
}

pub fn recursive_loop(input: Value, texts: &mut Vec<OcrCondifence>) -> Vec<OcrCondifence> {
    if let Some(result) = input.as_array() {
        if result.len() > 0 {
            if result.len() == 2 && result[0].is_string() && result[1].is_number() {
                let text = result[0].as_str().unwrap().to_string();
                let confidence_score = result[1].as_number().unwrap().as_f64().unwrap();
                texts.push(OcrCondifence {
                    text,
                    confidence: confidence_score * 100.0,
                    image: true,
                });
            }
            for input in result {
                recursive_loop(input.clone(), texts);
            }
        }
    }
    return texts.clone();
}

pub fn analyze_ocr(response_ocr: Value) -> Option<OcrCondifence> {
    if let Some(ok) = response_ocr.get("ok") {
        if ok.is_boolean() && ok.as_bool().unwrap() == true {
            if let Some(result) = response_ocr.get("result") {
                let results = recursive_loop(result.clone(), &mut vec![]);
                if results.len() > 0 {
                    let found = results
                        .iter()
                        .find(|x| detect_bs58(x.text.clone()).is_some());
                    if let Some(found) = found {
                        return Some(OcrCondifence {
                            text: detect_bs58(found.text.clone()).unwrap(),
                            ..found.clone()
                        });
                    }
                }
            }
        }
    }
    return None;
}

pub fn analyze_ocr_concatenated(response_ocr: Value) -> Option<String> {
    if let Some(ok) = response_ocr.get("ok") {
        if ok.is_boolean() && ok.as_bool().unwrap() == true {
            let mut texts: Vec<OcrCondifence> = vec![];
            if let Some(result) = response_ocr.get("result") {
                let results = recursive_loop(result.clone(), &mut vec![]);
                let mut current_text = "".to_string();
                for result in results {
                    current_text = format!("{}{}", current_text, result.text);
                }
                return Some(current_text);
            }
        }
    }
    return None;
}

pub fn analyze_text(response_ocr: Value) -> Vec<OcrCondifence> {
    let mut texts: Vec<OcrCondifence> = vec![];
    if let Some(ok) = response_ocr.get("ok") {
        if ok.is_boolean() && ok.as_bool().unwrap() == true {
            if let Some(result) = response_ocr.get("result") {
                texts = recursive_loop(result.clone(), &mut vec![]);
            }
        }
    }
    return texts;
}

fn canonical_solana(address: &str) -> bool {
    match bs58::decode(address).into_vec() {
        Ok(decoded) if decoded.len() == 32 => bs58::encode(&decoded).into_string() == address,
        _ => false,
    }
}

fn extract_solana_tokens_with_prefix2(text: &str, lengths: &[usize], prefix: &str) -> Vec<String> {
    let base58_alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut tokens = HashSet::new();

    // Since our text is ASCII (only alphanumeric characters), we can safely slice it.
    for &length in lengths {
        if text.len() < length {
            continue;
        }
        for i in 0..=text.len() - length {
            let candidate = &text[i..i + length];
            if candidate.starts_with(prefix)
                && candidate.chars().all(|c| base58_alphabet.contains(c))
                && canonical_solana(candidate)
            {
                tokens.insert(candidate.to_string());
            }
        }
    }
    tokens.into_iter().collect()
}

/// Synchronously checks the token supply for multiple token mint addresses
/// using a batch JSON RPC call. It constructs a list of requests, sends them in
/// a single HTTP POST, and returns the responses mapped to the corresponding tokens.
pub async fn check_tokens_supply_batch(
    token_mints: &[String],
) -> Result<HashMap<String, Option<Value>>, Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let url = env::var("RPC_ENDPOINT").expect("RPC endpoint is required");
    let client = Client::new();

    let mut payloads = Vec::new();
    let mut id_to_token = HashMap::new();
    for (i, token) in token_mints.iter().enumerate() {
        let request_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": i,
            "method": "getTokenSupply",
            "params": [token]
        });
        payloads.push(request_payload);
        id_to_token.insert(i, token.clone());
    }

    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&payloads)
        .send()
        .await?;

    let responses: Vec<Value> = response.json().await?;
    let mut results = HashMap::new();

    for resp in responses {
        if let Some(id) = resp.get("id").and_then(|v| v.as_u64()) {
            let token = id_to_token.get(&(id as usize)).cloned().unwrap_or_default();
            let supply_info = resp.get("result").and_then(|r| r.get("value")).cloned();
            results.insert(token, supply_info);
        }
    }

    Ok(results)
}

pub async fn bruteforce_text(
    input_text: &str,
) -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
    tracing::info!("Bruteforcing text - Original text:\n{}\n", input_text);

    // Remove all whitespace (spaces, newlines, etc.)
    let re_whitespace = Regex::new(r"\s+")?;
    let input_text = re_whitespace.replace_all(input_text, "");
    // Remove all non-alphanumeric characters (retain only A-Z, a-z, 0-9)
    let re_non_alnum = Regex::new(r"[^A-Za-z0-9]")?;
    let input_text = re_non_alnum.replace_all(&input_text, "");
    let cleaned_text = input_text.to_string();

    // println!("Cleaned text:\n{}\n", cleaned_text);

    // Extract candidate tokens (allowing tokens of length 43 or 44)
    let tokens = extract_solana_tokens_with_prefix2(&cleaned_text, &[43, 44], "");
    // println!("Found candidate tokens:");
    // for token in &tokens {
    //     println!("{}", token);
    // }

    let start = Instant::now();
    let token_supply_results = check_tokens_supply_batch(&tokens).await?;

    let mut valid_tokens = Vec::new();
    for (token, supply_info) in token_supply_results {
        if let Some(info) = supply_info {
            if let Some(amount_str) = info.get("amount").and_then(|v| v.as_str()) {
                if let Ok(amount) = amount_str.parse::<u64>() {
                    if amount > 0 {
                        valid_tokens.push((token, info));
                    }
                }
            }
        }
    }

    // println!("\nValid Solana token addresses with token supply:");
    let mut has_supply = Vec::new();
    if !valid_tokens.is_empty() {
        for (token, supply) in valid_tokens {
            // println!("Token: {}", token);
            // println!("Supply Info: {}", supply);
            has_supply.push(token);
        }
    } else {
        tracing::info!("No valid Solana token addresses found with token supply.");
    }

    let elapsed = start.elapsed();
    // println!("\nTime elapsed: {} ms", elapsed.as_millis());
    // println!("Tokens with supply: {:?}", has_supply);

    if has_supply.len() > 0 {
        return Ok(Some(has_supply[0].clone()));
    }
    return Ok(None);
}
