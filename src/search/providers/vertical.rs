//! Vertical engines: news wire, scholarly indexes, code and community sources.
//! These carry the strongest verification signal, so they are parsed carefully.

use super::{between, clean, enc, fetch, json_at, json_str_at, mk};
use crate::config::Config;
use crate::http::Http;
use crate::search::SearchResult;
use crate::util;
use anyhow::{anyhow, Result};

fn strip_cdata(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix("<![CDATA[").unwrap_or(t);
    let t = t.strip_suffix("]]>").unwrap_or(t);
    t.to_string()
}

fn xml_field(block: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    between(block, &start, &end).map(|s| strip_cdata(s))
}

// ---------------------------------------------------------------------------
// Google News RSS (free, no key, very wide press coverage)
// ---------------------------------------------------------------------------

pub fn google_news(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let locales: [(&str, &str, &str); 2] = [
        ("zh-CN", "CN", "CN:zh-Hans"),
        ("en-US", "US", "US:en"),
    ];
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (hl, gl, ceid) in locales {
        let url = format!("https://news.google.com/rss/search?q={}&hl={hl}&gl={gl}&ceid={ceid}", enc(q));
        match fetch(http, cfg, &url, &[]) {
            Ok(resp) => {
                for item in resp.body.split("<item>").skip(1) {
                    let title = xml_field(item, "title").unwrap_or_default();
                    let link = xml_field(item, "link").unwrap_or_default();
                    if title.is_empty() || !link.starts_with("http") {
                        continue;
                    }
                    let desc = xml_field(item, "description").unwrap_or_default();
                    let published = xml_field(item, "pubDate");
                    let source_name = xml_field(item, "source").unwrap_or_default();
                    let source_url = between(item, "<source url=\"", "\"").unwrap_or("");
                    let display_url = if source_url.starts_with("http") { source_url.to_string() } else { link.clone() };
                    let mut r = mk("googlenews", &title, &display_url, &desc);
                    r.published = published;
                    if !source_name.is_empty() {
                        r.extra = Some(format!("source:{source_name}"));
                    }
                    // keep the wire story link as the citable URL when we have a publisher domain
                    if !source_url.starts_with("http") {
                        r.url = link.clone();
                    }
                    out.push(r);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
            Err(e) => errors.push(format!("googlenews({ceid}): {e}")),
        }
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!(errors.join("; ")));
    }
    out.truncate(limit);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Crossref (DOI metadata)
// ---------------------------------------------------------------------------

pub fn crossref(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://api.crossref.org/works?query={}&rows={}&select=DOI,title,URL,container-title,issued,author,abstract,type,is-referenced-by-count&mailto=ves-agent@example.com",
        enc(q),
        limit
    );
    let resp = fetch(http, cfg, &url, &[])?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let items = json_at(&v, &["message", "items"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("crossref response missing message.items"))?;
    let mut out = Vec::new();
    for item in items {
        let title = json_str_at(item, &["title"]).unwrap_or_default();
        let doi = json_str_at(item, &["DOI"]).unwrap_or_default();
        let url = json_str_at(item, &["URL"])
            .filter(|u| u.starts_with("http"))
            .unwrap_or_else(|| format!("https://doi.org/{doi}"));
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let venue = json_str_at(item, &["container-title"]).unwrap_or_default();
        let year = item
            .get("issued")
            .and_then(|i| i.get("date-parts"))
            .and_then(|d| d.get(0))
            .and_then(|d| d.get(0))
            .and_then(|y| y.as_i64())
            .map(|y| y.to_string())
            .unwrap_or_default();
        let authors = item
            .get("author")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .take(3)
                    .filter_map(|a| {
                        let family = a.get("family").and_then(|x| x.as_str()).unwrap_or("");
                        let given = a.get("given").and_then(|x| x.as_str()).unwrap_or("");
                        Some(format!("{given} {family}").trim().to_string())
                    })
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let cited = item.get("is-referenced-by-count").and_then(|c| c.as_i64()).unwrap_or(0);
        let abstract_text = json_str_at(item, &["abstract"]).map(|a| clean(&a)).unwrap_or_default();
        let (abstract_short, _) = util::truncate_chars(&abstract_text, 260);
        let mut snippet = vec![venue.clone(), year.clone(), authors.clone()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        if cited > 0 {
            snippet.push_str(&format!(" · cited by {cited}"));
        }
        if !abstract_short.is_empty() {
            snippet.push_str(" — ");
            snippet.push_str(&abstract_short);
        }
        let mut r = mk("crossref", &title, &url, &snippet);
        r.published = if year.is_empty() { None } else { Some(year) };
        r.extra = Some(format!("doi:{doi}"));
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("crossref returned no usable works"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// OpenAlex (open scholarly graph with citation counts)
// ---------------------------------------------------------------------------

pub fn openalex(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://api.openalex.org/works?search={}&per-page={}&mailto=ves-agent@example.com",
        enc(q),
        limit
    );
    let resp = fetch(http, cfg, &url, &[])?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let items = json_at(&v, &["results"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("openalex response missing results"))?;
    let mut out = Vec::new();
    for item in items {
        let title = json_str_at(item, &["display_name"]).unwrap_or_default();
        let doi = json_str_at(item, &["doi"]).unwrap_or_default();
        let url = json_str_at(item, &["primary_location", "landing_page_url"])
            .filter(|u| u.starts_with("http"))
            .or_else(|| {
                let oa = json_str_at(item, &["open_access", "oa_url"]);
                oa.filter(|u| u.starts_with("http"))
            })
            .unwrap_or_else(|| doi.clone());
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let year = json_str_at(item, &["publication_year"]).unwrap_or_default();
        let cited = item.get("cited_by_count").and_then(|c| c.as_i64()).unwrap_or(0);
        let venue = json_str_at(item, &["primary_location", "source", "display_name"]).unwrap_or_default();
        let author = json_str_at(item, &["authorships", "0", "author", "display_name"]).unwrap_or_default();
        let mut snippet = vec![venue, year.clone(), author]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        if cited > 0 {
            snippet.push_str(&format!(" · cited by {cited}"));
        }
        let mut r = mk("openalex", &title, &url, &snippet);
        r.published = if year.is_empty() { None } else { Some(year) };
        r.extra = Some(format!("cited_by:{cited}"));
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("openalex returned no usable works"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// arXiv (preprints, Atom feed)
// ---------------------------------------------------------------------------

pub fn arxiv(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://export.arxiv.org/api/query?search_query=all:{}&start=0&max_results={}",
        enc(q),
        limit
    );
    let resp = fetch(http, cfg, &url, &[])?;
    let mut out = Vec::new();
    for entry in resp.body.split("<entry>").skip(1) {
        let title = xml_field(entry, "title").unwrap_or_default();
        let id = xml_field(entry, "id").unwrap_or_default();
        if title.is_empty() || !id.starts_with("http") {
            continue;
        }
        let summary = xml_field(entry, "summary").unwrap_or_default();
        let published = xml_field(entry, "published");
        let authors: Vec<String> = entry
            .split("<author>")
            .skip(1)
            .filter_map(|a| xml_field(a, "name"))
            .take(3)
            .collect();
        let mut snippet = authors.join(", ");
        if !summary.is_empty() {
            let (short, _) = util::truncate_chars(&clean(&summary), 240);
            if !snippet.is_empty() {
                snippet.push_str(" — ");
            }
            snippet.push_str(&short);
        }
        let mut r = mk("arxiv", &title, &id.replace("http://", "https://"), &snippet);
        r.published = published;
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("arxiv returned no entries"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Hacker News (Algolia API: fast, reliable, good for tech claim provenance)
// ---------------------------------------------------------------------------

pub fn hackernews(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://hn.algolia.com/api/v1/search?query={}&hitsPerPage={}&tags=story",
        enc(q),
        limit
    );
    let resp = fetch(http, cfg, &url, &[])?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let hits = json_at(&v, &["hits"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("hn response missing hits"))?;
    let mut out = Vec::new();
    for hit in hits {
        let object_id = json_str_at(hit, &["objectID"]).unwrap_or_default();
        let title = json_str_at(hit, &["title"])
            .or_else(|| json_str_at(hit, &["story_title"]))
            .unwrap_or_else(|| format!("Hacker News item {object_id}"));
        let url = json_str_at(hit, &["url"])
            .filter(|u| u.starts_with("http"))
            .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={object_id}"));
        let points = hit.get("points").and_then(|p| p.as_i64()).unwrap_or(0);
        let comments = hit.get("num_comments").and_then(|p| p.as_i64()).unwrap_or(0);
        let body = json_str_at(hit, &["story_text"])
            .or_else(|| json_str_at(hit, &["comment_text"]))
            .map(|t| clean(&t))
            .unwrap_or_default();
        let (short, _) = util::truncate_chars(&body, 220);
        let mut snippet = format!("{points} points · {comments} comments");
        if !short.is_empty() {
            snippet.push_str(" — ");
            snippet.push_str(&short);
        }
        let mut r = mk("hackernews", &title, &url, &snippet);
        r.published = json_str_at(hit, &["created_at"]);
        r.extra = Some(format!("hn:{}", object_id));
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("hacker news returned no stories"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// GitHub (implementation evidence; often TLS-reset in restricted networks)
// ---------------------------------------------------------------------------

pub fn github(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!("https://api.github.com/search/repositories?q={}&per_page={}", enc(q), limit);
    let resp = fetch(
        http,
        cfg,
        &url,
        &[
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
        ],
    )?;
    if resp.status == 403 {
        return Err(anyhow!("github returned 403 (rate limited or blocked)"));
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let items = json_at(&v, &["items"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("github response missing items"))?;
    let mut out = Vec::new();
    for item in items {
        let full = json_str_at(item, &["full_name"]).unwrap_or_default();
        let url = json_str_at(item, &["html_url"]).unwrap_or_default();
        if full.is_empty() || !url.starts_with("http") {
            continue;
        }
        let desc = json_str_at(item, &["description"]).unwrap_or_default();
        let stars = item.get("stargazers_count").and_then(|s| s.as_i64()).unwrap_or(0);
        let lang = json_str_at(item, &["language"]).unwrap_or_default();
        let snippet = format!("{desc} · ★{stars} · {lang}");
        let mut r = mk("github", &full, &url, &snippet);
        r.published = json_str_at(item, &["updated_at"]);
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("github returned no repositories"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Stack Exchange (practitioner answers)
// ---------------------------------------------------------------------------

pub fn stack_exchange(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://api.stackexchange.com/2.3/search/advanced?order=desc&sort=relevance&q={}&site=stackoverflow&pagesize={}&filter=default",
        enc(q),
        limit
    );
    let resp = fetch(http, cfg, &url, &[])?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let items = json_at(&v, &["items"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("stackexchange response missing items"))?;
    let mut out = Vec::new();
    for item in items {
        let title = json_str_at(item, &["title"]).unwrap_or_default();
        let link = json_str_at(item, &["link"]).unwrap_or_default();
        if title.is_empty() || !link.starts_with("http") {
            continue;
        }
        let score = item.get("score").and_then(|s| s.as_i64()).unwrap_or(0);
        let answers = item.get("answer_count").and_then(|s| s.as_i64()).unwrap_or(0);
        let accepted = item.get("is_answered").and_then(|s| s.as_bool()).unwrap_or(false);
        let tags = item
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let snippet = format!(
            "{score} votes · {answers} answers{} · {tags}",
            if accepted { " · accepted" } else { "" }
        );
        let mut r = mk("stackexchange", &clean(&title), &link, &snippet);
        r.published = item.get("creation_date").and_then(|d| d.as_i64()).map(util::iso8601);
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("stackexchange returned no questions"));
    }
    Ok(out)
}
