use anyhow::{Context, Result, ensure};
use stashfinder_core::{Action, PROTOCOL_VERSION, Request, Status, read_frame, write_frame};
use std::{mem::size_of, time::Duration};
use tokio::{
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
    time::{sleep, timeout},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER, TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
    core::{HSTRING, PWSTR},
};

pub const DEFAULT_PIPE: &str = r"\\.\pipe\StashFinder.M0";
pub const IO_TIMEOUT: Duration = Duration::from_secs(3);

pub fn current_user_sid() -> Result<String> {
    // SAFETY: Query-only token; aligned Windows-sized buffer; the token and allocated
    // SID string are released on every path.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let result = (|| -> Result<String> {
            let mut length = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
            ensure!(length > 0, "token user size unavailable");
            let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                length,
                &mut length,
            )?;
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut sid = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid)?;
            let value = sid.to_string();
            LocalFree(Some(HLOCAL(sid.0.cast())));
            Ok(value?)
        })();
        CloseHandle(token)?;
        result
    }
}

pub fn create_pipe(name: &str, owner_sid: &str, first: bool) -> Result<NamedPipeServer> {
    ensure!(
        name.starts_with(r"\\.\pipe\StashFinder."),
        "invalid local pipe name"
    );
    ensure!(
        owner_sid.starts_with("S-1-")
            && owner_sid
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-'),
        "invalid account SID"
    );
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{owner_sid})"));
    // SAFETY: Windows copies the descriptor into the kernel object before this call
    // returns. The descriptor is freed even if pipe creation fails.
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &sddl,
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(2)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            );
        LocalFree(Some(HLOCAL(descriptor.0)));
        Ok(result?)
    }
}

pub async fn status(name: &str) -> Result<Status> {
    timeout(IO_TIMEOUT, async {
        let mut client = loop {
            // SECURITY_IDENTIFICATION prevents server impersonation of the client.
            match ClientOptions::new().security_qos_flags(0x10000).open(name) {
                Ok(client) => break client,
                Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                    sleep(Duration::from_millis(30)).await
                }
                Err(error) => return Err(error.into()),
            }
        };
        let request = Request {
            version: PROTOCOL_VERSION,
            id: 1,
            action: Action::Status,
        };
        write_frame(&mut client, &serde_json::to_vec(&request)?).await?;
        let response: Status = serde_json::from_slice(&read_frame(&mut client).await?)?;
        ensure!(
            response.version == PROTOCOL_VERSION && response.id == request.id,
            "response version or request ID mismatch"
        );
        Ok(response)
    })
    .await
    .context("service did not respond within 3 seconds")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn restricts_pipe_to_owner_and_rejects_duplicate_server() {
        let name = format!(r"\\.\pipe\StashFinder.acl-{}", std::process::id());
        let sid = current_user_sid().unwrap();
        let _server = create_pipe(&name, &sid, true).unwrap();
        assert!(create_pipe(&name, &sid, true).is_err());
        assert!(ClientOptions::new().open(&name).is_ok());
        assert!(create_pipe(&name, "S-1-1-0)(A;;GA;;;WD", false).is_err());
        assert!(create_pipe(r"\\remote\pipe\bad", &sid, false).is_err());
    }

    #[tokio::test]
    async fn unconfigured_ordinary_user_is_denied() {
        // SAFETY: Read-only check of the current effective token.
        if unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() } {
            eprintln!("ordinary-user denial case requires an unelevated test process");
            return;
        }
        let name = format!(r"\\.\pipe\StashFinder.denied-{}", std::process::id());
        let _server = create_pipe(&name, "S-1-0-0", true).unwrap();
        let error = ClientOptions::new().open(&name).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
    }
}
