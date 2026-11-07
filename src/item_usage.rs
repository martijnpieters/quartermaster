//! What an item's work has spent: tokens and the API-equivalent cost across its sessions
//! (sub-agents included) and its asides. A lower bound when a session expired before the ledger
//! tallied it; partial when a model has no price.

use crate::error::{QmError, Res};
use crate::price_table::PriceTable;
use crate::ruby_format::format_f;
use crate::ruby_num::{sum, Num};
use crate::session_log::{Session, SessionLog};
use crate::transcript_usage::Buckets;
use crate::usage_ledger::UsageLedger;
use crate::work_item::WorkItem;

pub struct Row {
    pub id: String,
    pub aside: bool,
    pub when: Option<String>,
    pub known: bool,
    pub agents: i64,
    pub tokens: i64,
    pub cost: Num,
    pub unpriced: Vec<String>,
    pub models: Vec<(String, i64)>,
}

pub struct ItemUsage {
    pub rows: Vec<Row>,
}

fn tokens(buckets: &Buckets) -> i64 {
    buckets.values().filter_map(|k| k.as_object()).flat_map(|k| k.values()).filter_map(|v| v.as_i64()).sum()
}

fn models(buckets: &Buckets) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = vec![];
    for (bucket, kinds) in buckets {
        let model = bucket.split(':').next().unwrap_or("").to_string();
        let spent: i64 = kinds.as_object().map(|k| k.values().filter_map(|v| v.as_i64()).sum()).unwrap_or(0);
        match out.iter_mut().find(|(m, _)| *m == model) {
            Some((_, n)) => *n += spent,
            None => out.push((model, spent)),
        }
    }
    out
}

fn row(session: &Session, aside: bool, ledger: &mut UsageLedger, prices: &PriceTable) -> Result<Row, QmError> {
    let usage = ledger.session_usage(session)?;
    let (cost, unpriced) = prices.cost(&usage.buckets);
    let mut names: Vec<String> = vec![];
    for b in &unpriced {
        let m = b.split(':').next().unwrap_or("").to_string();
        if !names.contains(&m) {
            names.push(m);
        }
    }
    Ok(Row {
        id: session.id.clone(),
        aside,
        when: session.last_ts_opt(),
        known: usage.known,
        agents: usage.agents,
        tokens: tokens(&usage.buckets),
        cost,
        unpriced: names,
        models: models(&usage.buckets),
    })
}

impl ItemUsage {
    pub fn for_item(item: &WorkItem, log: &SessionLog, ledger: &mut UsageLedger, prices: &PriceTable) -> Res<ItemUsage> {
        let mut rows = vec![];
        for s in log.item_sessions(item)? {
            rows.push(row(&s, false, ledger, prices)?);
        }
        let mut asides: Vec<Session> = log.sessions().into_iter().filter(|s| s.aside_for().as_deref() == Some(item.slug.as_str())).collect();
        asides.sort_by_key(|s| s.started_ts());
        asides.reverse();
        for s in &asides {
            rows.push(row(s, true, ledger, prices)?);
        }
        Ok(ItemUsage { rows })
    }

    pub fn tokens(&self) -> i64 {
        self.rows.iter().map(|r| r.tokens).sum()
    }
    pub fn cost(&self) -> Num {
        sum(self.rows.iter().map(|r| r.cost))
    }
    pub fn lower_bound(&self) -> bool {
        self.rows.iter().any(|r| !r.known)
    }
    pub fn unpriced(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for r in &self.rows {
            for u in &r.unpriced {
                if !out.contains(u) {
                    out.push(u.clone());
                }
            }
        }
        out
    }
    pub fn partial(&self) -> bool {
        !self.unpriced().is_empty()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// "1.2M tok · ~$14", with "≥" when sessions are missing and "· partial" for an unpriced model.
    pub fn line(&self) -> String {
        if self.rows.iter().all(|r| !r.known) {
            return "usage unknown · sessions expired".into();
        }
        let bound = if self.lower_bound() { "≥ " } else { "" };
        let mut parts = vec![format!("{bound}{} tok", human_tokens(self.tokens())), format!("{}{}", if bound.is_empty() { "~" } else { bound }, dollars(self.cost()))];
        if self.partial() {
            parts.push("partial".into());
        }
        parts.join(" · ")
    }
}

pub fn human_tokens(count: i64) -> String {
    // Thresholds sit where the smaller unit would round up to "1000k" / "1000.0M".
    if count >= 999_950_000 {
        format!("{}B", format_f(count as f64 / 1e9, 1))
    } else if count >= 999_500 {
        format!("{}M", format_f(count as f64 / 1e6, 1))
    } else if count >= 1_000 {
        format!("{}k", format_f(count as f64 / 1e3, 0))
    } else {
        count.to_string()
    }
}

pub fn dollars(amount: Num) -> String {
    match amount {
        Num::Int(i) if i >= 10 => format!("${i}"),
        Num::Float(f) if f >= 10.0 => format!("${}", f.round() as i128),
        other => format!("${}", format_f(other.to_f64(), 2)),
    }
}
