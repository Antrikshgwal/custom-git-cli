use clap::Parser;
use colored::{Color, Colorize};
use rand::Rng;
use reqwest::header::{ACCEPT, HeaderMap, USER_AGENT};
use serde::Deserialize;
use std::error::Error;
use std::fmt;
use std::process;

#[derive(Debug)]
enum AppError {
    Request(reqwest::Error),
    HttpStatus {
        status: reqwest::StatusCode,
        message: String,
    },
    Json(serde_json::Error),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Request(err) => write!(f, "Request failed: {}", err),
            AppError::HttpStatus { status, message } => {
                write!(f, "GitHub API error ({}): {}", status, message)
            }
            AppError::Json(err) => write!(f, "Failed to parse response: {}", err),
        }
    }
}

impl Error for AppError {}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Request(err)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Json(err)
    }
}

#[derive(Parser, Debug, Deserialize)]
struct Args {
    // Input username
    #[arg(required(true), value_parser = validate_username)]
    username: String,
}

#[derive(Debug, Deserialize)]
struct Event {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    actor: Actor,
    repo: Repo,
    payload: serde_json::Value,
    public: bool,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct Actor {
    id: u64,
    login: String,
    display_login: String,
    gravatar_id: String,
    url: String,
    avatar_url: String,
}

#[derive(Debug, Deserialize)]
struct Repo {
    id: u64,
    name: String,
    url: String,
}

#[derive(Debug)]
struct ParsedEvent {
    repo: String,
    event_type: String,
    artifact: String,
    ref_action: String,
    ref_resource: String,
    count: Option<u64>,
    public: bool,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct CommitCommentPayload {
    action: Option<String>,
    comment: Option<CommitComment>,
}

#[derive(Debug, Deserialize)]
struct CommitComment {
    html_url: Option<String>,
    path: Option<String>,
    commit_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreatePayload {
    #[serde(rename = "ref")]
    ref_field: Option<String>,
    ref_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeletePayload {
    #[serde(rename = "ref")]
    ref_field: Option<String>,
    ref_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscussionPayload {
    action: Option<String>,
    discussion: Option<Discussion>,
}

#[derive(Debug, Deserialize)]
struct Discussion {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ForkPayload {
    action: Option<String>,
    forkee: Option<Forkee>,
}

#[derive(Debug, Deserialize)]
struct Forkee {
    full_name: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IssuePayload {
    action: Option<String>,
    issue: Option<IssueLike>,
}

#[derive(Debug, Deserialize)]
struct IssueLike {
    title: Option<String>,
    number: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MemberPayload {
    action: Option<String>,
    member: Option<Member>,
}

#[derive(Debug, Deserialize)]
struct Member {
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PullRequestPayload {
    action: Option<String>,
    pull_request: Option<IssueLike>,
    number: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PullRequestReviewPayload {
    action: Option<String>,
    pull_request: Option<IssueLike>,
}

#[derive(Debug, Deserialize)]
struct PushPayload {
    #[serde(rename = "ref")]
    ref_field: Option<String>,
    head: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WatchPayload {
    action: Option<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    let args = Args::parse();

    let response = fetch_events(&args.username)?;
    let parsed: Vec<ParsedEvent> = response.into_iter().filter_map(parse_event).collect();
    println!("Output:");
    for event in &parsed {
        println!("- {}", format_event(event));
    }

    Ok(())
}

fn parse_event(event: Event) -> Option<ParsedEvent> {
    let Event {
        event_type,
        repo,
        payload,
        public,
        created_at,
        ..
    } = event;

    let event_type = event_type;
    let (action, resource, artifact, count) = match (event_type.as_str(), payload) {
        ("CommitCommentEvent", payload) => {
            let payload: CommitCommentPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "created".to_string());
            let resource = payload
                .comment
                .and_then(|comment| comment.html_url.or(comment.path).or(comment.commit_id))
                .unwrap_or_else(|| "commit comment".to_string());
            let artifact = "comment".to_string();
            (action, resource, artifact, None)
        }
        ("CreateEvent", payload) => {
            let payload: CreatePayload = serde_json::from_value(payload).ok()?;
            let action = "created".to_string();
            let artifact = payload
                .ref_type
                .clone()
                .unwrap_or_else(|| "repository".to_string());
            let resource = payload
                .ref_field
                .or(payload.ref_type)
                .unwrap_or_else(|| "repository".to_string());
            (action, resource, artifact, None)
        }
        ("DeleteEvent", payload) => {
            let payload: DeletePayload = serde_json::from_value(payload).ok()?;
            let action = "deleted".to_string();
            let artifact = payload
                .ref_type
                .clone()
                .unwrap_or_else(|| "ref".to_string());
            let resource = payload
                .ref_field
                .or(payload.ref_type)
                .unwrap_or_else(|| "ref".to_string());
            (action, resource, artifact, None)
        }
        ("DiscussionEvent", payload) => {
            let payload: DiscussionPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "created".to_string());
            let resource = payload
                .discussion
                .and_then(|discussion| discussion.title)
                .unwrap_or_else(|| "discussion".to_string());
            let artifact = "discussion".to_string();
            (action, resource, artifact, None)
        }
        ("ForkEvent", payload) => {
            let payload: ForkPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "forked".to_string());
            let resource = payload
                .forkee
                .and_then(|forkee| forkee.full_name.or(forkee.name))
                .unwrap_or_else(|| "fork".to_string());
            let artifact = "fork".to_string();
            (action, resource, artifact, None)
        }
        ("IssueCommentEvent", payload) => {
            let payload: IssuePayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "created".to_string());
            let resource = issue_resource(payload.issue, "issue");
            let artifact = "comment".to_string();
            (action, resource, artifact, None)
        }
        ("IssuesEvent", payload) | ("IssueEvent", payload) => {
            let payload: IssuePayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "opened".to_string());
            let resource = issue_resource(payload.issue, "issue");
            let artifact = "issue".to_string();
            (action, resource, artifact, None)
        }
        ("MemberEvent", payload) => {
            let payload: MemberPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "added".to_string());
            let resource = payload
                .member
                .and_then(|member| member.login)
                .unwrap_or_else(|| "member".to_string());
            let artifact = "member".to_string();
            (action, resource, artifact, None)
        }
        ("PullRequestEvent", payload) => {
            let payload: PullRequestPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "opened".to_string());
            let resource = issue_resource(payload.pull_request, "pull request");
            let resource = if resource == "pull request" {
                payload
                    .number
                    .map(|number| format!("#{}", number))
                    .unwrap_or(resource)
            } else {
                resource
            };
            let artifact = "PR".to_string();
            (action, resource, artifact, None)
        }
        ("PullRequestReviewEvent", payload) => {
            let payload: PullRequestReviewPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "created".to_string());
            let resource = issue_resource(payload.pull_request, "pull request");
            let artifact = "review".to_string();
            (action, resource, artifact, None)
        }
        ("PushEvent", payload) => {
            let payload: PushPayload = serde_json::from_value(payload).ok()?;
            let PushPayload {
                ref_field,
                head,
                size,
            } = payload;
            let action = "pushed".to_string();
            let resource = ref_field
                .map(shorten_ref)
                .or(head)
                .unwrap_or_else(|| "ref".to_string());
            let artifact = "commit".to_string();
            (action, resource, artifact, size)
        }
        ("WatchEvent", payload) => {
            let payload: WatchPayload = serde_json::from_value(payload).ok()?;
            let action = payload.action.unwrap_or_else(|| "started".to_string());
            let resource = "repository".to_string();
            let artifact = "star".to_string();
            (action, resource, artifact, None)
        }
        _ => return None,
    };

    Some(ParsedEvent {
        repo: repo.name,
        event_type,
        artifact,
        ref_action: action,
        ref_resource: resource,
        count,
        public,
        created_at,
    })
}

fn fetch_events(username: &str) -> Result<Vec<Event>, AppError> {
    let url = format!("https://api.github.com/users/{}/events", username);

    let client = reqwest::blocking::Client::new();

    let response = client
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .header(USER_AGENT, "my-rust-app")
        .send()?;

    let status = response.status();
    let headers = response.headers().clone();
    let body = response.text()?;

    if !status.is_success() {
        let message = format_http_error(status, &headers, &body);
        return Err(AppError::HttpStatus { status, message });
    }

    let events: Vec<Event> = serde_json::from_str(&body)?;
    Ok(events)
}

fn format_http_error(status: reqwest::StatusCode, headers: &HeaderMap, body: &str) -> String {
    let remaining = header_value(headers, "X-RateLimit-Remaining");
    let reset = header_value(headers, "X-RateLimit-Reset");

    if status == reqwest::StatusCode::FORBIDDEN && remaining.as_deref() == Some("0") {
        if let Some(reset) = reset {
            return format!("Rate limit exceeded. Try again after unix time {}.", reset);
        }
        return "Rate limit exceeded. Try again later.".to_string();
    }

    let trimmed = body.trim();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let message = json
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Request failed");
        let docs = json
            .get("documentation_url")
            .and_then(|value| value.as_str());

        let mut friendly = if status == reqwest::StatusCode::NOT_FOUND && message == "Not Found" {
            "User not found. Check the username.".to_string()
        } else {
            message.to_string()
        };

        if let Some(docs) = docs {
            friendly.push_str(&format!(" (see: {})", docs));
        }

        return friendly
    }

    if trimmed.is_empty() {
        format!("Request failed with status {}", status)
    } else {
        trimmed.to_string()
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

fn format_event(event: &ParsedEvent) -> String {
    match event.event_type.as_str() {
        "PushEvent" => {
            let count = event.count.unwrap_or(0);
            let commit_word = if count == 1 { "commit" } else { "commits" };
            let verb = paint("Pushed");
            let action = if count > 0 {
                paint(&format!("{} {}", count, commit_word))
            } else {
                paint("commits")
            };
            let repo = paint(&event.repo);
            format!("{} {} to {}", verb, action, repo)
        }
        "WatchEvent" => {
            let verb = paint("Starred");
            let repo = paint(&event.repo);
            format!("{} {}", verb, repo)
        }
        "IssuesEvent" | "IssueEvent" => {
            let verb = paint(&capitalize_first(&event.ref_action));
            let repo = paint(&event.repo);
            if event.ref_action == "opened" {
                let action = paint("new issue");
                format!("{} a {} in {}", verb, action, repo)
            } else {
                let action = paint("issue");
                format!("{} an {} in {}", verb, action, repo)
            }
        }
        "IssueCommentEvent" => {
            let verb = paint("Commented");
            let issue_label = if event.ref_resource == "issue" {
                "issue".to_string()
            } else {
                format!("issue {}", event.ref_resource)
            };
            let action = paint(&issue_label);
            let repo = paint(&event.repo);
            format!("{} on {} in {}", verb, action, repo)
        }
        "PullRequestEvent" => {
            let verb = paint(&capitalize_first(&event.ref_action));
            let action = paint("pull request");
            let repo = paint(&event.repo);
            if event.ref_action == "opened" {
                format!("{} a {} in {}", verb, action, repo)
            } else {
                format!("{} a {} in {}", verb, action, repo)
            }
        }
        "PullRequestReviewEvent" => {
            let verb = if event.ref_action == "created" {
                "Reviewed".to_string()
            } else {
                capitalize_first(&event.ref_action)
            };
            let verb = paint(&verb);
            let action = paint("pull request review");
            let repo = paint(&event.repo);
            format!("{} a {} in {}", verb, action, repo)
        }
        "CommitCommentEvent" => {
            let verb = paint("Commented");
            let commit_label = if event.ref_resource == "commit comment" {
                "commit".to_string()
            } else {
                format!("commit {}", event.ref_resource)
            };
            let action = paint(&commit_label);
            let repo = paint(&event.repo);
            format!("{} on {} in {}", verb, action, repo)
        }
        "DiscussionEvent" => {
            let verb = paint(&capitalize_first(&event.ref_action));
            let action = paint("discussion");
            let repo = paint(&event.repo);
            format!("{} a {} in {}", verb, action, repo)
        }
        "ForkEvent" => {
            let verb = paint("Forked");
            let repo = paint(&event.repo);
            if event.ref_resource != "fork" {
                let target = paint(&event.ref_resource);
                format!("{} {} to {}", verb, repo, target)
            } else {
                format!("{} {}", verb, repo)
            }
        }
        "CreateEvent" => {
            let verb = paint("Created");
            let repo = paint(&event.repo);
            if event.artifact == "repository" {
                let action = paint("repository");
                format!("{} {} {}", verb, action, repo)
            } else {
                let action = paint(&event.artifact);
                let resource = paint(&event.ref_resource);
                format!("{} {} {} in {}", verb, action, resource, repo)
            }
        }
        "DeleteEvent" => {
            let verb = paint("Deleted");
            let action = paint(&event.artifact);
            let resource = paint(&event.ref_resource);
            let repo = paint(&event.repo);
            format!("{} {} {} in {}", verb, action, resource, repo)
        }
        "MemberEvent" => {
            let verb = paint(&capitalize_first(&event.ref_action));
            let member = paint(&event.ref_resource);
            let repo = paint(&event.repo);
            format!("{} {} to {}", verb, member, repo)
        }
        _ => {
            let verb = paint(&capitalize_first(&event.ref_action));
            let resource = paint(&event.ref_resource);
            let repo = paint(&event.repo);
            format!("{} {} in {}", verb, resource, repo)
        }
    }
}

fn paint(value: &str) -> String {
    value.color(random_color()).to_string()
}

fn random_color() -> Color {
    let colors = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::BrightRed,
        Color::BrightGreen,
        Color::BrightYellow,
        Color::BrightBlue,
        Color::BrightMagenta,
        Color::BrightCyan,
    ];
    let mut rng = rand::thread_rng();
    colors[rng.gen_range(0..colors.len())]
}

fn capitalize_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn shorten_ref(value: String) -> String {
    if let Some(stripped) = value.strip_prefix("refs/heads/") {
        return stripped.to_string();
    }
    if let Some(stripped) = value.strip_prefix("refs/tags/") {
        return stripped.to_string();
    }

    value
}

fn issue_resource(issue: Option<IssueLike>, fallback: &str) -> String {
    if let Some(issue) = issue {
        if let Some(title) = issue.title {
            return title;
        }
        if let Some(number) = issue.number {
            return format!("#{}", number);
        }
    }

    fallback.to_string()
}

fn validate_username(value: &str) -> Result<String, String> {
    let len = value.len();
    let ok_len = (1..=39).contains(&len);
    let ok_chars = value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let no_edge_dash = !value.starts_with('-') && !value.ends_with('-');
    let no_double_dash = !value.contains("--");

    if ok_len && ok_chars && no_edge_dash && no_double_dash {
        Ok(value.to_string())
    } else {
        Err("Invalid GitHub username".to_string())
    }
}
