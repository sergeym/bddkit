use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub polling: PollingOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingOptions {
    pub timeout: Duration,
    pub interval: Duration,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OptionsLayer {
    pub polling: Option<PollingOptionsLayer>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PollingOptionsLayer {
    pub timeout_secs: Option<u64>,
    pub interval_ms: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            polling: PollingOptions {
                timeout: Duration::from_secs(5),
                interval: Duration::from_millis(100),
            },
        }
    }
}

impl Options {
    pub fn apply(&self, layer: &OptionsLayer) -> Result<Self, String> {
        let mut resolved = self.clone();
        if let Some(polling) = &layer.polling {
            if let Some(value) = polling.timeout_secs {
                resolved.polling.timeout = Duration::from_secs(value);
            }
            if let Some(value) = polling.interval_ms {
                resolved.polling.interval = Duration::from_millis(value);
            }
        }
        resolved.validate()?;
        Ok(resolved)
    }

    fn validate(&self) -> Result<(), String> {
        if self.polling.timeout.is_zero() {
            return Err("options.polling.timeout_secs must be positive".into());
        }
        if self.polling.interval.is_zero() {
            return Err("options.polling.interval_ms must be positive".into());
        }
        if self.polling.interval > self.polling.timeout {
            return Err("options.polling.interval_ms must not exceed timeout_secs".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_five_seconds_and_one_hundred_milliseconds() {
        let options = Options::default();
        assert_eq!(options.polling.timeout, Duration::from_secs(5));
        assert_eq!(options.polling.interval, Duration::from_millis(100));
    }

    #[test]
    fn a_partial_layer_keeps_unspecified_values() {
        let layer = OptionsLayer {
            polling: Some(PollingOptionsLayer {
                timeout_secs: Some(12),
                interval_ms: None,
            }),
        };
        let options = Options::default().apply(&layer).unwrap();
        assert_eq!(options.polling.timeout, Duration::from_secs(12));
        assert_eq!(options.polling.interval, Duration::from_millis(100));
    }

    #[test]
    fn invalid_polling_values_are_rejected() {
        for layer in [
            PollingOptionsLayer {
                timeout_secs: Some(0),
                interval_ms: None,
            },
            PollingOptionsLayer {
                timeout_secs: None,
                interval_ms: Some(0),
            },
            PollingOptionsLayer {
                timeout_secs: Some(1),
                interval_ms: Some(1001),
            },
        ] {
            let error = Options::default()
                .apply(&OptionsLayer {
                    polling: Some(layer),
                })
                .unwrap_err();
            assert!(error.contains("polling"), "{error}");
        }
    }
}
