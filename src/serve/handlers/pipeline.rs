//! Pipeline / opportunities read REST handlers for `bastion serve` (BW.3.A).
//!
//! Read-only (D25) — these routes never mutate any brain file. They project the
//! business sub-brain's opportunity markdown (`business/docs/opportunities/*.md`
//! + `business/docs/leads/*.md`) into a typed "sales pipeline" of opportunities,
//!   plus the canonical stage vocabulary parsed from `business/docs/pipeline.md`.
//!
//! # Routes
//! - `GET /api/pipeline` — the stage vocabulary + a summary per opportunity,
//!   sorted by stage order (index in the vocab; unknown/none last) then title.
//! - `GET /api/pipeline/{slug}` — the full projection for one opportunity,
//!   including frontmatter (contacts, actions, links), the research brief parsed
//!   from the body's first ```json fence, and the raw body markdown.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`parse_stages`], [`parse_opportunity`], [`extract_json_brief`],
//! [`to_summary`], [`stage_rank`], [`valid_slug`], and the frontmatter/JSON
//! parsers are pure — unit-tested directly, no filesystem access. [`get_pipeline`]
//! and [`get_pipeline_opportunity`] are thin async shells: they resolve the HQ
//! brain root (`resolve_workspace_root` → `find_brain_root`), read the
//! `business/docs` tree under `web::block`, and hand the raw markdown to the pure
//! functions.
//!
//! # HQ/business root resolution
//! `business/` lives at the HQ brain root (a sibling of `brain.toml`), so the
//! root is resolved exactly like `handlers/board.rs` / `handlers/attention.rs`:
//! `resolve_workspace_root(None, None, &registry)` picks a starting path, then
//! `mev::brain::config::find_brain_root` walks up to the directory containing
//! `brain.toml`. `business/docs/...` is read relative to that root.
//!
//! # Error mapping
//! | Condition | Status | Code |
//! |---|---|---|
//! | Unknown/invalid `{slug}` (no matching file, or a slug with path separators) | 404 | `C002` |
//! | Brain root unresolvable (no `brain.toml`) | 500 | `C010` |
//! | `web::block` failure | 500 | `C010` |
//!
//! A missing/unreadable `business/` tree is **not** an error for the list route —
//! it degrades to empty `stages`/`opportunities` (read projection, D25).

use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    ContactDto, ErrorPayload, OpportunityActionDto, OpportunityDetailDto, OpportunitySummaryDto,
    PipelineDto, ProspectLeadDto, ResearchBriefDto,
};

use mev::brain::config::find_brain_root;

/// Files skipped when enumerating an opportunities/leads directory (index pages,
/// not opportunities themselves).
const SKIP_FILES: [&str; 2] = ["index.md", "readme.md"];

// ── Raw frontmatter / brief parse structs (internal, not on the wire) ─────────

/// The subset of an opportunity file's YAML frontmatter this projection reads.
/// Every field is optional (`title` falls back to the slug) so a minimal file
/// with only a title — or none at all — still parses.
#[derive(Debug, Default, Deserialize)]
struct RawFrontmatter {
    title: Option<String>,
    kind: Option<String>,
    stage: Option<String>,
    source: Option<String>,
    url: Option<String>,
    #[serde(default)]
    links: Vec<String>,
    last_contact: Option<String>,
    next_action: Option<String>,
    research_ref: Option<String>,
    #[serde(default)]
    contacts: Vec<RawContact>,
    #[serde(default)]
    actions: Vec<RawAction>,
}

#[derive(Debug, Default, Deserialize)]
struct RawContact {
    name: Option<String>,
    role: Option<String>,
    #[serde(default)]
    emails: Vec<String>,
    #[serde(default)]
    whatsapp: Vec<String>,
    #[serde(default)]
    phones: Vec<String>,
    #[serde(default)]
    links: Vec<String>,
    note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAction {
    at: Option<String>,
    kind: Option<String>,
    note: Option<String>,
}

/// The union of a CompanyBrief and a ProspectingResult, as they appear in the
/// body's first ```json fence (raw EN.4.A output). Presence of `company_name`
/// discriminates a company brief; otherwise it is treated as a prospecting result.
#[derive(Debug, Default, Deserialize)]
struct RawBrief {
    company_name: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    recent_developments: Vec<String>,
    #[serde(default)]
    pain_points: Vec<String>,
    #[serde(default)]
    outreach_hooks: Vec<String>,
    #[serde(default)]
    sources: Vec<String>,
    vertical: Option<String>,
    #[serde(default)]
    common_pain_points: Vec<String>,
    #[serde(default)]
    prospects: Vec<RawProspect>,
}

#[derive(Debug, Default, Deserialize)]
struct RawProspect {
    #[serde(default)]
    name: String,
    #[serde(default)]
    pillar: String,
    #[serde(default)]
    pain_points: Vec<String>,
    outreach_hook: Option<String>,
    source: Option<String>,
}

// ── Pure core ────────────────────────────────────────────────────────────────

/// Split a markdown document into its (optional) YAML frontmatter block and the
/// body that follows.
///
/// A frontmatter block is present only when the very first line is exactly `---`;
/// the block runs up to the next line that is exactly `---`. Returns
/// `(Some(yaml), body)` in that case, or `(None, full)` when there is no
/// well-formed leading fence (the whole document is then the body).
pub fn split_frontmatter(markdown: &str) -> (Option<&str>, &str) {
    let rest = match markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
    {
        Some(r) => r,
        None => return (None, markdown),
    };

    // Find the closing fence: a line that is exactly `---`.
    let Some(fence_abs) = find_fence_line(rest) else {
        // No closing fence — treat the whole document as body (no frontmatter).
        return (None, markdown);
    };

    // `fence_abs` is the byte offset (within `rest`) of the `-` of the closing
    // `---` line. The YAML is everything before it; the body is everything after
    // the fence line's trailing newline.
    let yaml = &rest[..fence_abs];
    let after_fence = &rest[fence_abs..];
    let body = after_fence
        .strip_prefix("---\n")
        .or_else(|| after_fence.strip_prefix("---\r\n"))
        .or_else(|| after_fence.strip_prefix("---"))
        .unwrap_or("");
    (Some(yaml), body)
}

/// Return the byte offset within `slice` of the start of the first line that is
/// exactly `---` (ignoring trailing `\r`), or `None`.
fn find_fence_line(slice: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in slice.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

/// Parse the canonical stage vocabulary from `pipeline.md`.
///
/// Looks for the `## Stages` heading, then the first following line that carries
/// backtick-wrapped tokens (e.g.
/// `` `identified` → `researching` → … ``) and returns those tokens in order.
/// Returns an empty vec when no such section/line is found.
pub fn parse_stages(pipeline_md: &str) -> Vec<String> {
    let mut in_section = false;
    for line in pipeline_md.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            in_section = heading.trim().eq_ignore_ascii_case("stages");
            continue;
        }
        if in_section && trimmed.contains('`') {
            let tokens = backtick_tokens(trimmed);
            if !tokens.is_empty() {
                return tokens;
            }
        }
    }
    Vec::new()
}

/// Extract every `` `token` `` substring from `line`, in order.
fn backtick_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch != '`' {
            continue;
        }
        let content_start = start + 1;
        let mut content_end = None;
        for (i, c) in chars.by_ref() {
            if c == '`' {
                content_end = Some(i);
                break;
            }
        }
        if let Some(end) = content_end {
            let token = line[content_start..end].trim();
            if !token.is_empty() {
                out.push(token.to_string());
            }
        }
    }
    out
}

/// Extract and parse the research brief from an opportunity body's first
/// ```json fenced block. Returns `None` when there is no json fence or the
/// content does not parse as JSON.
///
/// The brief's `kind` is `"company"` when the JSON carries a `company_name`
/// field, else `"prospecting"`.
pub fn extract_json_brief(body: &str) -> Option<ResearchBriefDto> {
    let json = first_json_fence(body)?;
    let raw: RawBrief = serde_json::from_str(json).ok()?;

    let kind = if raw.company_name.is_some() {
        "company"
    } else {
        "prospecting"
    };

    Some(ResearchBriefDto {
        kind: kind.to_string(),
        company_name: raw.company_name,
        summary: raw.summary,
        recent_developments: raw.recent_developments,
        pain_points: raw.pain_points,
        outreach_hooks: raw.outreach_hooks,
        sources: raw.sources,
        vertical: raw.vertical,
        common_pain_points: raw.common_pain_points,
        prospects: raw
            .prospects
            .into_iter()
            .map(|p| ProspectLeadDto {
                name: p.name,
                pillar: p.pillar,
                pain_points: p.pain_points,
                outreach_hook: p.outreach_hook,
                source: p.source,
            })
            .collect(),
    })
}

/// Return the raw content of the first ```json fenced block in `body`, or `None`.
fn first_json_fence(body: &str) -> Option<&str> {
    // Find the opening fence line (```json, optionally with trailing spaces).
    let open_idx = body.find("```json")?;
    let after_open = &body[open_idx + "```json".len()..];
    // Skip to the end of the opening fence line.
    let content_start_rel = after_open.find('\n')? + 1;
    let content = &after_open[content_start_rel..];
    // Find the closing fence.
    let close_rel = content.find("```")?;
    Some(content[..close_rel].trim())
}

/// Parse one opportunity markdown file into an [`OpportunityDetailDto`].
///
/// Frontmatter is parsed as YAML (missing/malformed frontmatter degrades to
/// defaults); `title` falls back to `slug` and `kind` to `"company"`. The
/// research brief is parsed from the body's first ```json fence.
pub fn parse_opportunity(slug: &str, markdown: &str) -> OpportunityDetailDto {
    let (yaml, body) = split_frontmatter(markdown);
    let fm: RawFrontmatter = yaml
        .and_then(|y| serde_yaml::from_str(y).ok())
        .unwrap_or_default();

    let title = fm
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| slug.to_string());
    let kind = fm
        .kind
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| "company".to_string());

    let contacts = fm
        .contacts
        .into_iter()
        .map(|c| ContactDto {
            name: c.name,
            role: c.role,
            emails: c.emails,
            whatsapp: c.whatsapp,
            phones: c.phones,
            links: c.links,
            note: c.note,
        })
        .collect();

    let actions = fm
        .actions
        .into_iter()
        .map(|a| OpportunityActionDto {
            at: a.at.unwrap_or_default(),
            kind: a.kind.unwrap_or_default(),
            note: a.note.unwrap_or_default(),
        })
        .collect();

    let findings = extract_json_brief(body);

    let body_trimmed = body.trim();
    let body_markdown = if body_trimmed.is_empty() {
        None
    } else {
        Some(body_trimmed.to_string())
    };

    OpportunityDetailDto {
        slug: slug.to_string(),
        kind,
        title,
        source: fm.source,
        stage: fm.stage,
        last_contact: fm.last_contact,
        next_action: fm.next_action,
        url: fm.url,
        links: fm.links,
        research_ref: fm.research_ref,
        contacts,
        findings,
        actions,
        body_markdown,
    }
}

/// Project an [`OpportunityDetailDto`] down to its list-view [`OpportunitySummaryDto`].
pub fn to_summary(detail: &OpportunityDetailDto) -> OpportunitySummaryDto {
    OpportunitySummaryDto {
        slug: detail.slug.clone(),
        kind: detail.kind.clone(),
        title: detail.title.clone(),
        source: detail.source.clone(),
        stage: detail.stage.clone(),
        last_contact: detail.last_contact.clone(),
        next_action: detail.next_action.clone(),
        has_findings: detail.findings.is_some(),
    }
}

/// Rank an opportunity's stage against the stage vocabulary: the index of
/// `stage` in `stages`, or `usize::MAX` when the stage is `None` or unknown
/// (so it sorts last).
pub fn stage_rank(stage: Option<&str>, stages: &[String]) -> usize {
    match stage {
        Some(s) => stages.iter().position(|st| st == s).unwrap_or(usize::MAX),
        None => usize::MAX,
    }
}

/// Sort opportunity summaries in place by stage order (via [`stage_rank`]) then
/// title (case-insensitive).
pub fn sort_opportunities(items: &mut [OpportunitySummaryDto], stages: &[String]) {
    items.sort_by(|a, b| {
        let ra = stage_rank(a.stage.as_deref(), stages);
        let rb = stage_rank(b.stage.as_deref(), stages);
        ra.cmp(&rb)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

/// Is `slug` a safe single-segment file stem — no path separators, no `.`, no
/// NUL/backslash, non-empty? Guards the `{slug}` detail route against traversal
/// before any path is constructed.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && !slug.contains('/')
        && !slug.contains('\\')
        && !slug.contains('.')
        && !slug.contains('\0')
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// Build a 500 response from a `BlockingError`, mirroring the other handlers.
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Build a 500 response for a brain-root resolution failure, mirroring
/// `handlers/board.rs::brain_root_error_response`.
fn brain_root_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: message.to_string(),
    })
}

/// Build a 404 response for an unknown opportunity slug (no matching file, or a
/// slug that failed [`valid_slug`]) — one uniform shape so an invalid slug is
/// never distinguished from a genuinely absent one.
fn unknown_opportunity_response(slug: &str) -> HttpResponse {
    HttpResponse::NotFound().json(ErrorPayload {
        code: "C002".to_owned(),
        message: format!("unknown opportunity: {slug}"),
    })
}

/// The two directories opportunity files are read from, in list order:
/// `business/docs/opportunities` then `business/docs/leads`.
fn opportunity_dirs(brain_root: &Path) -> [PathBuf; 2] {
    let docs = brain_root.join("business").join("docs");
    [docs.join("opportunities"), docs.join("leads")]
}

/// Read `business/docs/pipeline.md` and parse its stage vocabulary. A missing or
/// unreadable file degrades to an empty vec (read projection, D25).
fn read_stages(brain_root: &Path) -> Vec<String> {
    let path = brain_root.join("business").join("docs").join("pipeline.md");
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_stages(&content),
        Err(_) => Vec::new(),
    }
}

/// Enumerate opportunity `*.md` files across both directories, parse each, and
/// return the details keyed by slug (first occurrence wins across dirs — an
/// `opportunities/x.md` shadows a `leads/x.md`). Skips `index.md`/`README.md`
/// and any non-`.md` file. Unreadable dirs/files are skipped.
fn read_all_opportunities(brain_root: &Path) -> Vec<OpportunityDetailDto> {
    let mut seen_slugs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();

    for dir in opportunity_dirs(brain_root) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if SKIP_FILES.iter().any(|s| file_name.eq_ignore_ascii_case(s)) {
                continue;
            }
            let is_md = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("md"))
                .unwrap_or(false);
            if !is_md {
                continue;
            }
            let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !seen_slugs.insert(slug.to_string()) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.push(parse_opportunity(slug, &content));
        }
    }

    out
}

/// Read one opportunity by slug, searching both directories in order. Returns
/// `None` when no `<slug>.md` exists in either.
fn read_one_opportunity(brain_root: &Path, slug: &str) -> Option<OpportunityDetailDto> {
    for dir in opportunity_dirs(brain_root) {
        let path = dir.join(format!("{slug}.md"));
        if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            return Some(parse_opportunity(slug, &content));
        }
    }
    None
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /api/pipeline` — stage vocabulary + one summary per opportunity.
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_pipeline(registry: web::Data<FileConfig>) -> HttpResponse {
    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<PipelineDto, String> {
        let root = find_brain_root(&start)
            .map_err(|e| format!("could not resolve brain root from {}: {e}", start.display()))?;
        let stages = read_stages(&root);
        let details = read_all_opportunities(&root);
        let mut opportunities: Vec<OpportunitySummaryDto> =
            details.iter().map(to_summary).collect();
        sort_opportunities(&mut opportunities, &stages);
        Ok(PipelineDto {
            stages,
            opportunities,
        })
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(msg)) => brain_root_error_response(msg),
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/pipeline/{slug}` — the full projection for one opportunity.
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware`.
pub async fn get_pipeline_opportunity(
    slug: web::Path<String>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    // The slug guard runs first (pure, cheap) so an invalid slug never touches
    // the filesystem and is indistinguishable from a genuinely absent one.
    let slug = slug.into_inner();
    if !valid_slug(&slug) {
        return unknown_opportunity_response(&slug);
    }

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));
    let slug_for_block = slug.clone();
    match web::block(move || -> Result<Option<OpportunityDetailDto>, String> {
        let root = find_brain_root(&start)
            .map_err(|e| format!("could not resolve brain root from {}: {e}", start.display()))?;
        Ok(read_one_opportunity(&root, &slug_for_block))
    })
    .await
    {
        Ok(Ok(Some(detail))) => HttpResponse::Ok().json(detail),
        Ok(Ok(None)) => unknown_opportunity_response(&slug),
        Ok(Err(msg)) => brain_root_error_response(msg),
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── split_frontmatter ────────────────────────────────────────────────────

    #[test]
    fn split_frontmatter_extracts_block_and_body() {
        let md = "---\ntitle: X\nkind: company\n---\n# Body\n\ntext\n";
        let (yaml, body) = split_frontmatter(md);
        assert_eq!(yaml, Some("title: X\nkind: company\n"));
        assert_eq!(body, "# Body\n\ntext\n");
    }

    #[test]
    fn split_frontmatter_no_fence_is_all_body() {
        let md = "# Body only\n";
        let (yaml, body) = split_frontmatter(md);
        assert_eq!(yaml, None);
        assert_eq!(body, "# Body only\n");
    }

    #[test]
    fn split_frontmatter_unterminated_fence_is_all_body() {
        let md = "---\ntitle: X\nno closing fence\n";
        let (yaml, body) = split_frontmatter(md);
        assert_eq!(yaml, None);
        assert_eq!(body, md);
    }

    // ── parse_stages ─────────────────────────────────────────────────────────

    const REAL_STAGES_MD: &str = "# Pipeline\n\n## Stages\n\n`identified` → `researching` → `contacted` → `conversation` → `proposal-sent` → `closed-won` → `closed-lost`\n\n---\n\n## Active Leads\n";

    #[test]
    fn parse_stages_reads_real_vocabulary_in_order() {
        let stages = parse_stages(REAL_STAGES_MD);
        assert_eq!(
            stages,
            vec![
                "identified",
                "researching",
                "contacted",
                "conversation",
                "proposal-sent",
                "closed-won",
                "closed-lost",
            ]
        );
    }

    #[test]
    fn parse_stages_missing_section_is_empty() {
        assert!(parse_stages("# Pipeline\n\nno stages here\n").is_empty());
    }

    #[test]
    fn parse_stages_section_without_backticks_is_empty() {
        assert!(parse_stages("## Stages\n\nplain prose, no tokens\n").is_empty());
    }

    // ── extract_json_brief — company vs prospecting detection ─────────────────

    const COMPANY_BODY: &str = "# Anthropic\n\nsome prose\n\n## Research Brief\n```json\n{\n  \"company_name\": \"Anthropic\",\n  \"summary\": \"An AI lab.\",\n  \"pain_points\": [\"scaling\"],\n  \"outreach_hooks\": [\"hook one\"],\n  \"recent_developments\": [\"IPO\"],\n  \"sources\": [\"https://example.com\"]\n}\n```\n";

    const PROSPECTING_BODY: &str = "# E-commerce\n\n## Research Brief\n```json\n{\n  \"vertical\": \"e-commerce\",\n  \"common_pain_points\": [\"WISMO\"],\n  \"prospects\": [\n    { \"name\": \"Shopify Owners\", \"pillar\": \"Interactive AI Assistants\", \"pain_points\": [\"tickets\"], \"outreach_hook\": \"hook\", \"source\": \"https://example.com\" }\n  ]\n}\n```\n";

    #[test]
    fn extract_json_brief_detects_company() {
        let brief = extract_json_brief(COMPANY_BODY).expect("should parse");
        assert_eq!(brief.kind, "company");
        assert_eq!(brief.company_name.as_deref(), Some("Anthropic"));
        assert_eq!(brief.summary.as_deref(), Some("An AI lab."));
        assert_eq!(brief.pain_points, vec!["scaling"]);
        assert_eq!(brief.outreach_hooks, vec!["hook one"]);
        assert_eq!(brief.recent_developments, vec!["IPO"]);
        assert_eq!(brief.sources, vec!["https://example.com"]);
        assert!(brief.prospects.is_empty());
    }

    #[test]
    fn extract_json_brief_detects_prospecting() {
        let brief = extract_json_brief(PROSPECTING_BODY).expect("should parse");
        assert_eq!(brief.kind, "prospecting");
        assert!(brief.company_name.is_none());
        assert_eq!(brief.vertical.as_deref(), Some("e-commerce"));
        assert_eq!(brief.common_pain_points, vec!["WISMO"]);
        assert_eq!(brief.prospects.len(), 1);
        let p = &brief.prospects[0];
        assert_eq!(p.name, "Shopify Owners");
        assert_eq!(p.pillar, "Interactive AI Assistants");
        assert_eq!(p.pain_points, vec!["tickets"]);
        assert_eq!(p.outreach_hook.as_deref(), Some("hook"));
        assert_eq!(p.source.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn extract_json_brief_none_when_no_fence() {
        assert!(extract_json_brief("# No brief here\n").is_none());
    }

    #[test]
    fn extract_json_brief_none_when_json_malformed() {
        let body = "```json\n{ not valid json ]\n```\n";
        assert!(extract_json_brief(body).is_none());
    }

    // ── parse_opportunity — full company fixture ──────────────────────────────

    const COMPANY_FIXTURE: &str = "---\ntype: Opportunity\ntitle: Anthropic\nkind: company\nstage: identified\nsource: RESEARCH_AGENT test run\nurl: https://www.anthropic.com\nlinks:\n  - https://www.anthropic.com\nresearch_ref: engine-rs-research-runs\ncontacts: []\nactions:\n  - { at: 2026-07-25, kind: research, note: \"Generated CompanyBrief\" }\n---\n\n# Anthropic\n\n## Research Brief\n```json\n{ \"company_name\": \"Anthropic\", \"summary\": \"An AI lab.\" }\n```\n";

    #[test]
    fn parse_opportunity_company_reads_frontmatter_and_findings() {
        let d = parse_opportunity("anthropic", COMPANY_FIXTURE);
        assert_eq!(d.slug, "anthropic");
        assert_eq!(d.kind, "company");
        assert_eq!(d.title, "Anthropic");
        assert_eq!(d.stage.as_deref(), Some("identified"));
        assert_eq!(d.source.as_deref(), Some("RESEARCH_AGENT test run"));
        assert_eq!(d.url.as_deref(), Some("https://www.anthropic.com"));
        assert_eq!(d.links, vec!["https://www.anthropic.com"]);
        assert_eq!(d.research_ref.as_deref(), Some("engine-rs-research-runs"));
        assert!(d.contacts.is_empty());
        assert_eq!(d.actions.len(), 1);
        assert_eq!(d.actions[0].at, "2026-07-25");
        assert_eq!(d.actions[0].kind, "research");
        assert_eq!(d.actions[0].note, "Generated CompanyBrief");
        let findings = d.findings.expect("company brief present");
        assert_eq!(findings.kind, "company");
        assert_eq!(findings.company_name.as_deref(), Some("Anthropic"));
        assert!(d.body_markdown.is_some());
    }

    // ── parse_opportunity — prospecting sweep fixture ─────────────────────────

    const PROSPECTING_FIXTURE: &str = "---\ntitle: E-commerce — Support Automation\nkind: prospecting-sweep\nstage: identified\nsource: RESEARCH_AGENT prospecting mode\ncontacts: []\nactions:\n  - { at: 2026-07-25, kind: research, note: \"Generated ProspectingResult\" }\n---\n\n# E-commerce\n\n## Research Brief\n```json\n{ \"vertical\": \"e-commerce\", \"common_pain_points\": [\"WISMO\"], \"prospects\": [ { \"name\": \"Shopify Owners\", \"pillar\": \"AI Assistants\" } ] }\n```\n";

    #[test]
    fn parse_opportunity_prospecting_detects_findings_kind() {
        let d = parse_opportunity("ecommerce-support-automation", PROSPECTING_FIXTURE);
        assert_eq!(d.kind, "prospecting-sweep");
        assert_eq!(d.title, "E-commerce — Support Automation");
        let findings = d.findings.expect("prospecting brief present");
        assert_eq!(findings.kind, "prospecting");
        assert_eq!(findings.vertical.as_deref(), Some("e-commerce"));
        assert_eq!(findings.prospects.len(), 1);
        assert_eq!(findings.prospects[0].name, "Shopify Owners");
    }

    // ── parse_opportunity — contacts-bearing synthetic fixture ────────────────

    const CONTACTS_FIXTURE: &str = "---\ntitle: Warm Lead\nkind: company\nstage: contacted\ncontacts:\n  - name: Jane Doe\n    role: Head of Ops\n    emails: [jane@example.com, j.doe@corp.com]\n    whatsapp: [\"+15551234567\"]\n    phones: [\"+15559876543\"]\n    links: [https://linkedin.com/in/jane]\n    note: Met at conference\n---\n\n# Warm Lead\n\nbody text\n";

    #[test]
    fn parse_opportunity_reads_structured_contacts() {
        let d = parse_opportunity("warm-lead", CONTACTS_FIXTURE);
        assert_eq!(d.contacts.len(), 1);
        let c = &d.contacts[0];
        assert_eq!(c.name.as_deref(), Some("Jane Doe"));
        assert_eq!(c.role.as_deref(), Some("Head of Ops"));
        assert_eq!(c.emails, vec!["jane@example.com", "j.doe@corp.com"]);
        assert_eq!(c.whatsapp, vec!["+15551234567"]);
        assert_eq!(c.phones, vec!["+15559876543"]);
        assert_eq!(c.links, vec!["https://linkedin.com/in/jane"]);
        assert_eq!(c.note.as_deref(), Some("Met at conference"));
        assert!(d.findings.is_none());
    }

    // ── parse_opportunity — minimal title-only fixture (defaults) ─────────────

    #[test]
    fn parse_opportunity_minimal_title_only_uses_defaults() {
        let md = "---\ntitle: Just A Title\n---\n\nbody\n";
        let d = parse_opportunity("just-a-title", md);
        assert_eq!(d.title, "Just A Title");
        assert_eq!(d.kind, "company"); // default
        assert!(d.stage.is_none());
        assert!(d.source.is_none());
        assert!(d.url.is_none());
        assert!(d.links.is_empty());
        assert!(d.contacts.is_empty());
        assert!(d.actions.is_empty());
        assert!(d.findings.is_none());
    }

    #[test]
    fn parse_opportunity_no_title_falls_back_to_slug() {
        let md = "---\nkind: company\n---\n\nbody\n";
        let d = parse_opportunity("my-slug", md);
        assert_eq!(d.title, "my-slug");
    }

    #[test]
    fn parse_opportunity_no_frontmatter_uses_all_defaults() {
        let d = parse_opportunity("bare", "# Bare\n\njust body\n");
        assert_eq!(d.title, "bare");
        assert_eq!(d.kind, "company");
        assert_eq!(d.body_markdown.as_deref(), Some("# Bare\n\njust body"));
    }

    // ── to_summary / has_findings ─────────────────────────────────────────────

    #[test]
    fn to_summary_projects_fields_and_has_findings() {
        let with = parse_opportunity("anthropic", COMPANY_FIXTURE);
        let s = to_summary(&with);
        assert_eq!(s.slug, "anthropic");
        assert_eq!(s.kind, "company");
        assert_eq!(s.title, "Anthropic");
        assert_eq!(s.stage.as_deref(), Some("identified"));
        assert!(s.has_findings);

        let without = parse_opportunity("minimal", "---\ntitle: M\n---\n\nbody\n");
        assert!(!to_summary(&without).has_findings);
    }

    // ── stage_rank / sort_opportunities ───────────────────────────────────────

    fn stages() -> Vec<String> {
        parse_stages(REAL_STAGES_MD)
    }

    #[test]
    fn stage_rank_orders_by_vocab_index_unknown_last() {
        let s = stages();
        assert_eq!(stage_rank(Some("identified"), &s), 0);
        assert_eq!(stage_rank(Some("contacted"), &s), 2);
        assert_eq!(stage_rank(Some("closed-lost"), &s), 6);
        assert_eq!(stage_rank(Some("not-a-stage"), &s), usize::MAX);
        assert_eq!(stage_rank(None, &s), usize::MAX);
    }

    fn summary(slug: &str, title: &str, stage: Option<&str>) -> OpportunitySummaryDto {
        OpportunitySummaryDto {
            slug: slug.to_string(),
            kind: "company".to_string(),
            title: title.to_string(),
            source: None,
            stage: stage.map(str::to_string),
            last_contact: None,
            next_action: None,
            has_findings: false,
        }
    }

    #[test]
    fn sort_opportunities_by_stage_then_title() {
        let s = stages();
        let mut items = vec![
            summary("z", "Zebra", Some("contacted")),
            summary("a", "Apple", Some("identified")),
            summary("b", "Banana", Some("identified")),
            summary("u", "Unknown Stage", Some("not-a-stage")),
            summary("n", "No Stage", None),
        ];
        sort_opportunities(&mut items, &s);
        let order: Vec<&str> = items.iter().map(|i| i.slug.as_str()).collect();
        // identified (Apple, Banana) → contacted (Zebra) → unknown/none (title-sorted).
        assert_eq!(order, vec!["a", "b", "z", "n", "u"]);
    }

    // ── valid_slug ─────────────────────────────────────────────────────────────

    #[test]
    fn valid_slug_accepts_plain_stems() {
        assert!(valid_slug("anthropic"));
        assert!(valid_slug("ecommerce-support-automation"));
        assert!(valid_slug("gui-brazilianportugui"));
    }

    #[test]
    fn valid_slug_rejects_traversal_and_separators() {
        assert!(!valid_slug(""));
        assert!(!valid_slug("../secret"));
        assert!(!valid_slug("a/b"));
        assert!(!valid_slug("a\\b"));
        assert!(!valid_slug("file.md"));
        assert!(!valid_slug("with\0nul"));
    }

    // ── error responses ────────────────────────────────────────────────────────

    #[test]
    fn unknown_opportunity_response_is_404_c002() {
        let resp = unknown_opportunity_response("ghost");
        assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn brain_root_error_response_is_500_c010() {
        let resp = brain_root_error_response("no brain.toml");
        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // ── I/O shell: read_stages / read_all_opportunities over a temp brain ──────

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("bastion_pipeline_test_{name}_{pid}_{id}"));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    /// Build a temp brain root with `business/docs/pipeline.md`, two
    /// opportunities, one lead, and an `index.md`/`README.md` that must be skipped.
    fn fixture_brain() -> TempDir {
        let tmp = TempDir::new("brain");
        let docs = tmp.path().join("business").join("docs");
        write(&docs.join("pipeline.md"), REAL_STAGES_MD);
        write(
            &docs.join("opportunities").join("anthropic.md"),
            COMPANY_FIXTURE,
        );
        write(&docs.join("opportunities").join("index.md"), "# index\n");
        write(
            &docs.join("opportunities").join("ecommerce.md"),
            PROSPECTING_FIXTURE,
        );
        write(&docs.join("leads").join("warm-lead.md"), CONTACTS_FIXTURE);
        write(&docs.join("leads").join("README.md"), "# readme\n");
        write(&docs.join("leads").join("scrape.py"), "print('hi')\n");
        tmp
    }

    #[test]
    fn read_stages_reads_vocabulary() {
        let tmp = fixture_brain();
        assert_eq!(read_stages(tmp.path())[0], "identified");
    }

    #[test]
    fn read_stages_missing_file_is_empty() {
        let tmp = TempDir::new("no-pipeline");
        assert!(read_stages(tmp.path()).is_empty());
    }

    #[test]
    fn read_all_opportunities_skips_index_readme_and_non_md() {
        let tmp = fixture_brain();
        let mut slugs: Vec<String> = read_all_opportunities(tmp.path())
            .into_iter()
            .map(|d| d.slug)
            .collect();
        slugs.sort();
        assert_eq!(slugs, vec!["anthropic", "ecommerce", "warm-lead"]);
    }

    #[test]
    fn read_one_opportunity_finds_across_both_dirs() {
        let tmp = fixture_brain();
        assert!(read_one_opportunity(tmp.path(), "anthropic").is_some());
        assert!(read_one_opportunity(tmp.path(), "warm-lead").is_some());
        assert!(read_one_opportunity(tmp.path(), "does-not-exist").is_none());
    }

    #[test]
    fn read_all_opportunities_missing_business_dir_is_empty() {
        let tmp = TempDir::new("empty-brain");
        assert!(read_all_opportunities(tmp.path()).is_empty());
    }
}
