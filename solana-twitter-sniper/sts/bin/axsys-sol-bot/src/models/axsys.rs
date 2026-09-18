// Represents a twitter user's account information in a minimal format

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdHandle {
    pub id: String,     // The account ID of the user who was replied to
    pub handle: String, // The account handle (Without @) of the user who was replied to
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniUser {
    pub id: String,     // The user's account ID
    pub handle: String, // The user's account handle (Without @)

    // Contains the user's profile information
    pub profile: Profile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniTweetBodyUrl {
    pub name: String, // The display name of the URL
    pub url: String,  // The original URL value of the URL
    pub tco: String,  // The Twitter t.co wrapped URL variant of the URL
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniTweetBodyMention {
    pub id: String,     // The mentioned user's account ID
    pub name: String,   // The mentioned user's display name
    pub handle: String, // The mentioned user's account handle (Without @)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniTweetBody {
    pub text: String, // The plain text of the tweet body

    // The list of clickable URLs contained in the tweet
    pub urls: Vec<TwitterMiniTweetBodyUrl>,

    // The list of mentioned users in the tweet
    pub mentions: Vec<TwitterMiniTweetBodyMention>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniTweetBodyMediaProxied {
    pub images: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniTweetBodyMedia {
    pub images: Vec<String>, // The list of image URLs in the tweet
    pub videos: Vec<String>, // The list of video URLs in the tweet

    // Note! Proxied media will only be available in certain scenarios
    // Fall back to using normal media URLs if proxied URLs are not available
    pub proxied: Option<TwitterMiniTweetBodyMediaProxied>,
}

// Represents a twitter tweet's information in a minimal format
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwitterMiniTweet {
    pub id: String,      // The tweet's snowflake ID
    pub r#type: String,  // The type of the tweet
    pub created_at: u64, // The UNIX timestamp of when the tweet was created on Twitter's servers in milliseconds

    // The account information of the tweet's author
    pub author: TwitterMiniUser,

    // The subtweet (reply, quote, retweet) referenced by this tweet (if any)
    pub subtweet: Option<Box<TwitterMiniTweet>>,

    // Contains information about who the tweet is replying to (if any)
    pub reply: Option<IdHandle>,

    // Contains information about who the tweet is quoting (if any)
    pub quoted: Option<IdHandle>,

    // Contains the tweet's body information
    pub body: TwitterMiniTweetBody,

    // Contains information about the tweet's media
    pub media: TwitterMiniTweetBodyMedia,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub limit: u64,
    pub usage: u64,
    pub remaining: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanResponse {
    pub plan: Plan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchProfiles {
    pub plan: Plan,
    pub watched: HashMap<String, String>,
}

/// Add watched profile
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddWatchedProfilePayload {
    pub handle: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TweetEvent {
    pub r#type: String,
    pub tweet: TwitterMiniTweet,
}

impl TweetEvent {
    pub fn flatten_tweets(
        tweets: &mut Vec<TwitterMiniTweet>,
        current_tweets: &mut Vec<TwitterMiniTweet>,
    ) -> Vec<TwitterMiniTweet> {
        if tweets.len() > 0 {
            for tweet in tweets {
                current_tweets.push(tweet.clone());
                if let Some(subtweet) = &tweet.subtweet {
                    TweetEvent::flatten_tweets(&mut vec![*subtweet.clone()], current_tweets);
                }
            }
        }
        return current_tweets.clone();
    }
}
