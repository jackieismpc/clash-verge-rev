use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleOverrides {
    pub prepend: Vec<String>,
    pub append: Vec<String>,
    pub delete: Vec<String>,
}
fn canonical(rule: &str) -> String {
    let kind = rule.split(',').next().unwrap_or("").trim();
    if [
        "DOMAIN",
        "DOMAIN-SUFFIX",
        "DOMAIN-KEYWORD",
        "IP-CIDR",
        "IP-CIDR6",
        "SRC-IP-CIDR",
        "GEOSITE",
        "GEOIP",
        "RULE-SET",
        "MATCH",
        "NETWORK",
        "DST-PORT",
        "SRC-PORT",
    ]
    .contains(&kind)
    {
        rule.split(',').map(str::trim).collect::<Vec<_>>().join(",")
    } else {
        // Regex/logical payload whitespace can be significant; use exact deletion.
        rule.to_owned()
    }
}
fn terminal(rule: &str) -> bool {
    rule.split_once(',').is_some_and(|(kind, _)| kind.trim() == "MATCH")
}
impl RuleOverrides {
    pub fn validate(&self) -> Result<()> {
        if self.prepend.len() + self.append.len() + self.delete.len() > 10_000 {
            bail!("local rule overrides exceed 10000 entries");
        }
        for rule in self.prepend.iter().chain(&self.append).chain(&self.delete) {
            if rule.trim() != rule
                || rule.is_empty()
                || rule.len() > 8192
                || rule.chars().any(char::is_control)
                || !rule.contains(',')
            {
                bail!("local rule must be a nonempty single-line mihomo rule");
            }
        }
        if self.prepend.iter().any(|rule| terminal(rule)) {
            bail!("MATCH is a final fallback; place it last in append, not prepend");
        }
        let matches: Vec<_> = self
            .append
            .iter()
            .enumerate()
            .filter(|(_, rule)| terminal(rule))
            .collect();
        if matches.len() > 1
            || matches
                .first()
                .is_some_and(|(index, _)| *index + 1 != self.append.len())
        {
            bail!("append may contain only one MATCH, as its last rule");
        }
        Ok(())
    }
    pub fn apply(&self, yaml: &mut Value) -> Result<()> {
        self.validate()?;
        let root = yaml.as_mapping_mut().context("profile root must be a mapping")?;
        let key = Value::String("rules".into());
        let source: Vec<String> = match root.get(&key) {
            None | Some(Value::Null) => vec![],
            Some(Value::Sequence(rules)) => rules
                .iter()
                .map(|rule| {
                    rule.as_str()
                        .map(str::to_owned)
                        .context("profile rules must contain strings")
                })
                .collect::<Result<_>>()?,
            _ => bail!("profile rules must be a sequence"),
        };
        if self.prepend.is_empty() && self.append.is_empty() && self.delete.is_empty() {
            return Ok(());
        }
        let deleted: HashSet<_> = self.delete.iter().map(|rule| canonical(rule)).collect();
        let local_fallback = self.append.last().filter(|rule| terminal(rule));
        let mut fallback = None;
        let mut source_rules = vec![];
        for rule in source {
            if deleted.contains(&canonical(&rule)) {
                continue;
            }
            if terminal(&rule) {
                if fallback.is_some() {
                    bail!("source contains multiple MATCH rules; remove the extra fallback explicitly");
                }
                fallback = Some(rule);
            } else {
                if fallback.is_some() {
                    bail!("source contains rules after MATCH; repair its order before applying local rules");
                }
                source_rules.push(rule);
            }
        }
        let mut seen = HashSet::new();
        let mut merged = vec![];
        for rule in self
            .prepend
            .iter()
            .chain(&source_rules)
            .chain(self.append.iter().filter(|rule| !terminal(rule)))
        {
            if seen.insert(canonical(rule)) {
                merged.push(Value::String(rule.clone()));
            }
        }
        if let Some(rule) = local_fallback.or(fallback.as_ref()) {
            merged.push(Value::String(rule.clone()));
        }
        root.insert(key, Value::Sequence(merged));
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_rules_are_reachable_before_source_fallback_after_refresh() {
        let edits = RuleOverrides {
            prepend: vec!["DOMAIN,priority.example,REJECT".into()],
            append: vec!["DOMAIN,local.example,DIRECT".into()],
            delete: vec!["DOMAIN,deleted.example,Remote".into()],
        };
        let edits: RuleOverrides = serde_json::from_str(&serde_json::to_string(&edits).unwrap()).unwrap();
        for source in [
            "rules: ['DOMAIN, deleted.example, Remote', 'DOMAIN,priority.example,REJECT', 'MATCH,Remote']",
            "rules: ['DOMAIN,fresh.example,Remote', 'MATCH,Remote']",
        ] {
            let mut yaml: Value = serde_yaml_ng::from_str(source).unwrap();
            edits.apply(&mut yaml).unwrap();
            let rules: Vec<_> = yaml["rules"]
                .as_sequence()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(rules[0], "DOMAIN,priority.example,REJECT");
            assert_eq!(rules[rules.len() - 2], "DOMAIN,local.example,DIRECT");
            assert_eq!(rules.last(), Some(&"MATCH,Remote"));
            assert_eq!(
                rules.iter().filter(|v| **v == "DOMAIN,priority.example,REJECT").count(),
                1
            );
            assert!(!rules.iter().any(|v| v.contains("deleted.example")));
        }
    }
    #[test]
    fn explicit_local_fallback_replaces_source_match() {
        let edits = RuleOverrides {
            append: vec!["DOMAIN,local.example,DIRECT".into(), "MATCH,REJECT".into()],
            ..Default::default()
        };
        let mut yaml: Value =
            serde_yaml_ng::from_str("rules: ['DOMAIN,source.example,DIRECT', 'MATCH,Remote']").unwrap();
        edits.apply(&mut yaml).unwrap();
        assert_eq!(yaml["rules"][2].as_str(), Some("MATCH,REJECT"));
        assert_eq!(yaml["rules"].as_sequence().unwrap().len(), 3);
    }
    #[test]
    fn invalid_or_unreachable_rules_do_not_change_document() {
        for edits in [
            RuleOverrides {
                prepend: vec!["MATCH,DIRECT".into()],
                ..Default::default()
            },
            RuleOverrides {
                append: vec!["MATCH,DIRECT".into(), "DOMAIN,later.example,REJECT".into()],
                ..Default::default()
            },
            RuleOverrides {
                prepend: vec!["MATCH,DIRECT\nrules: []".into()],
                ..Default::default()
            },
        ] {
            let mut yaml: Value = serde_yaml_ng::from_str("rules: ['MATCH,DIRECT']").unwrap();
            let old = yaml.clone();
            assert!(edits.apply(&mut yaml).is_err());
            assert_eq!(yaml, old);
        }
        let edits = RuleOverrides {
            append: vec!["DOMAIN,local.example,DIRECT".into()],
            ..Default::default()
        };
        for text in [
            "rules: wrong",
            "rules: ['MATCH,DIRECT','DOMAIN,unreachable.example,DIRECT']",
        ] {
            let mut yaml: Value = serde_yaml_ng::from_str(text).unwrap();
            let old = yaml.clone();
            assert!(edits.apply(&mut yaml).is_err());
            assert_eq!(yaml, old);
        }
    }
}
