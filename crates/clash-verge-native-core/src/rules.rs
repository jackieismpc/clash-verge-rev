use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleOverrides {
    pub prepend: Vec<String>,
    pub append: Vec<String>,
    pub delete: Vec<String>,
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
        Ok(())
    }

    pub fn apply(&self, yaml: &mut Value) -> Result<()> {
        self.validate()?;
        let root = yaml.as_mapping_mut().context("profile root must be a mapping")?;
        let key = Value::String("rules".into());
        let rules = match root.get(&key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Sequence(rules)) => rules
                .iter()
                .map(|rule| {
                    rule.as_str()
                        .map(str::to_owned)
                        .context("profile rules must contain strings")
                })
                .collect::<Result<Vec<_>>>()?,
            _ => bail!("profile rules must be a sequence"),
        };
        // Deletions target source rules by exact text, including duplicate entries.
        let merged = self
            .prepend
            .iter()
            .cloned()
            .chain(rules.into_iter().filter(|rule| !self.delete.contains(rule)))
            .chain(self.append.iter().cloned())
            .map(Value::String)
            .collect();
        root.insert(key, Value::Sequence(merged));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn order_and_exact_delete_survive_refresh_and_persistence() {
        let edits = RuleOverrides {
            prepend: vec!["DOMAIN,local.example.invalid,DIRECT".into()],
            append: vec!["MATCH,DIRECT".into()],
            delete: vec!["MATCH,Remote".into()],
        };
        let saved = serde_json::to_string(&edits).unwrap();
        let edits: RuleOverrides = serde_json::from_str(&saved).unwrap();
        for source in [
            "rules: [\"MATCH,Remote\", \"MATCH,Remote\"]",
            "rules: [\"DOMAIN,fresh.example.invalid,Remote\", \"MATCH,Remote\"]",
        ] {
            let mut yaml: Value = serde_yaml_ng::from_str(source).unwrap();
            edits.apply(&mut yaml).unwrap();
            let rules = yaml["rules"].as_sequence().unwrap();
            assert_eq!(
                rules.first().unwrap().as_str(),
                Some("DOMAIN,local.example.invalid,DIRECT")
            );
            assert_eq!(rules.last().unwrap().as_str(), Some("MATCH,DIRECT"));
            assert!(!rules.iter().any(|rule| rule.as_str() == Some("MATCH,Remote")));
        }
    }
    #[test]
    fn rejects_invalid_edits_without_changing_document() {
        let edits = RuleOverrides {
            prepend: vec!["MATCH,DIRECT\nrules: []".into()],
            ..Default::default()
        };
        let mut yaml: Value = serde_yaml_ng::from_str("rules: [\"MATCH,DIRECT\"]").unwrap();
        let old = yaml.clone();
        assert!(edits.apply(&mut yaml).is_err());
        assert_eq!(yaml, old);
        assert!(
            RuleOverrides::default()
                .apply(&mut serde_yaml_ng::from_str("rules: wrong").unwrap())
                .is_err()
        );
    }
}
