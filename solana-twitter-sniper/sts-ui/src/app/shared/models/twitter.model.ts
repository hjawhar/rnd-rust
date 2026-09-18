// export type AddTwitterWatchedPayload = {
//     handle: string
// }

// export type GetTwitterWatchedPlan = {
//     limit: number,
//     usage: number,
//     remaining: number
// }

// export type GetTwitterWatchedResponse = {
//     plan: GetTwitterWatchedPlan,
//     watched: { [key: string]: string }
// }


// Represents a twitter user's account information in complete format
export interface TwitterUser {
    id: string; // The user's account ID
    handle: string; // The user's account handle (Without @)
    private: boolean; // If the user's account is currently private or not
    verified: boolean; // If the user's account is blue check mark verified or not
    sensitive: boolean; // If the user's account is marked to contain sensitive information or not
    restricted: boolean; // If the user's account is currently restricted for unusual activity or not
    joined_at: number; // The UNIX timestamp of when the user joined Twitter in milliseconds

    // Contains the user's profile information
    profile: {
        name: string; // The user's profile display name
        location: null | string; // The user's profile location (if one is set)
        avatar: null | string; // The user's custom profile avatar url (if one is set)
        banner: null | string; // The user's custom profile banner url (if one is set)
        pinned: string[]; // The list of pinned tweet IDs (if any are pinned) on the user's profile

        // Contains the user's profile custom link information (if one is set)
        url: null | {
            name: string; // The display name of the link
            url: string; // The original URL of the link
            tco: string; // The Twitter t.co wrapped URL variant of the link
        };

        // Contains the user's profile description information
        description: {
            text: string; // The plain text of the description

            // The list of clickable URLs contained in the description
            urls: {
                name: string; // The display name of the URL
                url: string; // The original URL value of the URL
                tco: string; // The Twitter t.co wrapped URL variant of the URL
            }[];
        };
    };

    // Contains the user's profile metrics information
    metrics: {
        likes: number; // The number of liked tweets by the user
        media: number; // The number of media (images, videos, gifs) posted by the user
        tweets: number; // The number of tweets posted by the user
        friends: number; // The number of friends of the user (People who follow the user and the user follows them back)
        followers: number; // The number of followers of the user
        following: number; // The number of accounts followed by the user
    };
}

// Represents a twitter tweet's information in complete format
export interface TwitterTweet {
    id: string; // The tweet's snowflake ID
    type: 'TWEET' | 'RETWEET' | 'QUOTE' | 'REPLY'; // The type of the tweet
    created_at: number; // The UNIX timestamp of when the tweet was created on Twitter's servers in milliseconds

    // The account information of the tweet's author
    author: TwitterUser;

    // The subtweet (reply, quote, retweet) referenced by this tweet (if any)
    subtweet: null | TwitterTweet;

    // Contains information about who the tweet is replying to (if any)
    reply: null | {
        id: string; // The account ID of the user who was replied to
        handle: string; // The account handle (Without @) of the user who was replied to
    };

    // Contains information about who the tweet is quoting (if any)
    quoted: null | {
        id: string; // The account ID of the user who was quoted
        handle: string; // The account handle (Without @) of the user who was quoted
    };

    // Contains the tweet's body information
    body: {
        text: string; // The plain text of the tweet body

        // The list of clickable URLs contained in the tweet
        urls: {
            name: string; // The display name of the URL
            url: string; // The original URL value of the URL
            tco: string; // The Twitter t.co wrapped URL variant of the URL
        }[];

        // The list of mentioned users in the tweet
        mentions: {
            id: string; // The mentioned user's account ID
            name: string; // The mentioned user's display name
            handle: string; // The mentioned user's account handle (Without @)
        }[];

        // The list of structured components contained in the tweet body for expanded rich / long tweets (if any)
        components: (
            | {
                // The `text` component represents represents a text part of a long tweet
                type: 'text';
                text: string; // The plain text of the component
                bold: boolean; // If the text is bold or not
                italics: boolean; // If the text is italics or not
            }
            | {
                // The `image` component represents represents an image part of a long tweet
                type: 'image';
                url: string;
            }
            | {
                // The `video` component represents represents a video part of a long tweet
                type: 'video';
                url: string;
            }
        )[];
    };

    // Contains information about the tweet's media
    media: {
        images: string[]; // The list of image URLs in the tweet
        videos: string[]; // The list of video URLs in the tweet
        thumbnails: string[]; // The list of video thumbnail URLs in the tweet

        // Note! Proxied media will only be available in certain scenarios
        // Fall back to using normal media URLs if proxied URLs are not available
        proxied: null | {
            images: string[]; // The list of proxied image URLs in the tweet
            thumbnails: string[]; // The list of proxied thumbnail URLs in the tweet
        };
    };

    // Contains the embedded Grok xAI conversation preview in the tweet (if any)
    grok: null | {
        id: string; // The ID of the Grok conversation

        // The preview information of the Grok conversation (Supports either an answer or generated images)
        conversation: {
            from: 'USER' | 'AGENT'; // Who is the sender of the message
            message: string; // The text of the message
            images: string[]; // The list of image URLs in the message
        }[];
    };

    // Contains information about an embedded card within the tweet (if any)
    card: null | {
        url: string; // The original URL of the card
        image: string; // The thumbnail image URL of the card
        title: string; // The title text of the card
        description: string; // The description text of the card
    };

    // Contains information about a poll within the tweet (if any)
    poll: null | {
        ends_at: number; // The UNIX timestamp of when the poll ends in milliseconds
        updated_at: number; // The UNIX timestamp of when the poll was last updated in milliseconds

        // The list of poll choices
        choices: {
            label: string; // The label text of the poll choice
            count: number; // The number of votes for the poll choice
        }[];
    };

    // Contains information about an article within the tweet (if any)
    article: null | {
        id: string; // The article's snowflake ID
        title: string; // The title text of the article
        thumbnail: null | string; // The thumbnail image URL of the article
        created_at: number; // The UNIX timestamp of when the article was created in milliseconds
        updated_at: number; // The UNIX timestamp of when the article was last updated in milliseconds

        // Contains the article's body information
        body: {
            text: string; // The plain text of the article body

            // The list of components represents the rich structured component within the article
            // RENDERING: Each component should be rendered as a separate section with double line vertical spacing
            components: (
                | {
                    // The `divider` component represents a divider between article sections
                    // Twitter / X user interface renders this divider as a horizontal gray line
                    type: 'divider';
                }

                // The `text` component represents some textual component within the article
                | {
                    type: 'text';

                    // The `header-one` variant is a large title which separates the article into sections
                    // The `header-two` variant is a smaller title which separates the article into subsections
                    // The `paragraph` variant is just regular paragraph text
                    // The `blockquote` variant is just paragraph text idented by a line to form a quoted block of text
                    // The `ordered-list` variant is an ordered list with numbering for each line
                    // The `unordered-list` variant is an unordered list with bullet points for each line
                    // The `latex-box` variant is a LaTeX equation box used to render mathematical equations (Ex: `$$a^2 + b^2 = c^2$$`)
                    // The `markdown-box` variant is a Markdown code block used to render code snippets (Ex: ```js\nconsole.log('Hello World!');\n```)
                    variant:
                    | 'header-one'
                    | 'header-two'
                    | 'paragraph'
                    | 'blockquote'
                    | 'ordered-list'
                    | 'unordered-list'
                    | 'latex-box'
                    | 'markdown-box';

                    // The list of lines represents the list items of this component
                    // RENDERING: Each line should be rendered as a separate line with single line vertical spacing
                    lines: {
                        // The plain text of this line
                        // The `styles` array contains unique styling information for parts of the text
                        // The `urls` array contains unique URLs for parts of the text
                        text: string;

                        // The list of styling applied to parts of the text identified by the `from` and `to` indexes
                        styles: {
                            from: number; // The starting index of the text which is styled
                            to: number; // The ending index of the text which is styled
                            text: 'bold' | 'italics' | 'strikethrough'; // The type of text styling applied
                        }[];

                        // The list of clickable hyperlink URLs identified by the `from` and `to` indexes
                        urls: {
                            from: number; // The starting index of the text which is a URL
                            to: number; // The ending index of the text which is a URL
                            url: string; // The clickable URL link of the text
                        }[];
                    }[];
                }

                // The `media` component represents some visual media within the article
                | {
                    type: 'media';

                    // The `image` variant is an image media type
                    // The `gif` variant is a GIF media type
                    // The `video` variant is a video media type
                    variant: 'image' | 'gif' | 'video';
                    url: string; // The media URL for this variant of the media
                    thumbnail: string; // The media thumbnail URL for this variant of the media
                    caption: null | string; // The associated caption text (if any)
                }

                // The `tweet` component represents an embedded tweet within the article
                | {
                    type: 'tweet';
                    tweet: {
                        id: string; // The tweet's snowflake ID
                        url: string; // The tweet's URL
                        object: null | TwitterTweet; // The tweet object (May need to be fetched separately if null)
                    };
                }
            )[];
        };
    };

    // Contains information about the tweet's metrics
    metrics: {
        likes: number; // The number of likes on the tweet
        quotes: number; // The number of quotes on the tweet
        replies: number; // The number of replies on the tweet
        retweets: number; // The number of retweets on the tweet
    };
}