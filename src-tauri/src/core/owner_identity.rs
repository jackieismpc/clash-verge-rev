use crate::utils::dirs;
use anyhow::{Context as _, Result};
use clash_verge_service_ipc::{OwnerCredentials, OwnerIdentity};
use std::path::Path;

pub(crate) fn current_owner_credentials() -> Result<OwnerCredentials> {
    current_owner_credentials_for_root(&dirs::app_home_dir()?)
}

pub(crate) fn current_owner_identity() -> OwnerIdentity {
    OwnerIdentity::Unix {
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
    }
}

pub(crate) fn current_owner_credentials_for_root(app_root: &Path) -> Result<OwnerCredentials> {
    let app_data_root = std::fs::canonicalize(app_root)
        .with_context(|| format!("failed to canonicalize application data root {app_root:?}"))?;

    let (identity, token) = (current_owner_identity(), None);

    Ok(OwnerCredentials {
        identity,
        app_data_dir: app_data_root.to_string_lossy().into_owned(),
        token,
    })
}

#[cfg(test)]
mod tests {
    use super::current_owner_credentials_for_root;
    use clash_verge_service_ipc::OwnerIdentity;

    #[test]
    fn unix_credentials_come_from_process_and_have_no_token() -> anyhow::Result<()> {
        let app_root = std::env::temp_dir();

        let credentials = current_owner_credentials_for_root(&app_root)?;

        assert_eq!(
            credentials.identity,
            OwnerIdentity::Unix {
                uid: unsafe { libc::geteuid() },
                gid: unsafe { libc::getegid() },
            }
        );
        assert_eq!(credentials.token, None);
        assert_eq!(
            std::path::PathBuf::from(credentials.app_data_dir),
            std::fs::canonicalize(app_root)?
        );
        Ok(())
    }
}
