//! The Windows service: `vork service install` registers `vork service-run
//! --config <file>` with the service manager, which starts it at boot. The
//! node logs to `vork.log` in its state directory, since a service has no console.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType,
    ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

const NAME: &str = "vork";
static CONFIG: OnceLock<PathBuf> = OnceLock::new();

pub fn install(config: &Path) -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .context("opening the service manager (run as administrator)")?;
    let info = ServiceInfo {
        name: OsString::from(NAME),
        display_name: OsString::from("vØrk"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec![OsString::from("service-run"), OsString::from("--config"), config.as_os_str().to_owned()],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let service = manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    service.set_description("Serverless, identity-addressed network over WireGuard")?;
    service.start::<&str>(&[])?;
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).context("opening the service manager (run as administrator)")?;
    let service = manager.open_service(NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        let _ = service.stop();
        for _ in 0..20 {
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    service.delete()?;
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

pub fn run_dispatcher(config: PathBuf) -> Result<()> {
    let _ = CONFIG.set(config);
    service_dispatcher::start(NAME, ffi_service_main).context("not started by the service manager")?;
    Ok(())
}

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(5),
        process_id: None,
    }
}

fn service_main(_args: Vec<OsString>) {
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(handle) = service_control_handler::register(NAME, handler) else { return };
    // Started again when its process ends unexpectedly: a self-update ends it so
    // (ADR-0028). Set here, as the service has the rights to.
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("sc.exe")
            // Every restart after 5 s: a day with several updates counted each as
            // a failure, and from the third the node was down a minute.
            .args(["failure", NAME, "reset=", "3600", "actions=", "restart/5000/restart/5000/restart/5000"])
            .creation_flags(0x0800_0000)
            .output();
    }
    let _ = handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN));
    let result = (|| -> Result<()> {
        let mut s = crate::settings(crate::RunArgs { config: CONFIG.get().cloned(), ..Default::default() })?;
        if s.log_file.is_none() {
            s.log_file = s.state.as_ref().map(|d| d.join("vork.log"));
        }
        tokio::runtime::Runtime::new()?.block_on(crate::run_node(s, Some(stop_rx)))
    })();
    if let Err(e) = &result {
        crate::node::log(format!("service stopped with an error: {e:#}"));
    }
    let _ = handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()));
}
