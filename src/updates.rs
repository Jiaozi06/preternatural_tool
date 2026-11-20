use std::ffi::c_void;
use std::fs;
use std::io;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::*;

const API_PATH: &str = "/Jiaozi06/preternatural_tool/main/update.json";
const MIRROR_PATH: &str =
    "/https://raw.githubusercontent.com/Jiaozi06/preternatural_tool/main/update.json";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(serde::Deserialize, Clone)]
pub struct OnlineManifest {
    pub version: String,
    pub release_url: String,
    pub download_url: String,
    pub mirror_download_url: String,
    pub changelog: Vec<String>,
}

struct Handle(*mut c_void);

impl Handle {
    fn from_raw(raw: *mut c_void, label: &str) -> io::Result<Self> {
        if raw.is_null() {
            Err(io::Error::other(format!(
                "{label}：{}",
                io::Error::last_os_error()
            )))
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

pub fn fetch_manifest(mirror: bool) -> Result<OnlineManifest, String> {
    request_manifest(mirror).map_err(|error| error.to_string())
}

pub fn download_update(url: &str, version: &str) -> Result<PathBuf, String> {
    let version_is_safe = !version.is_empty()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.');
    if !version_is_safe {
        return Err("更新版本号格式无效".into());
    }
    let (host, path) = if let Some(path) = url.strip_prefix("https://github.com/") {
        ("github.com", format!("/{path}"))
    } else if let Some(path) = url.strip_prefix("https://gh-proxy.com/https://github.com/") {
        ("gh-proxy.com", format!("/https://github.com/{path}"))
    } else {
        return Err("下载地址不是受支持的 GitHub 源".into());
    };
    let body = request_download(&host, &path).map_err(|error| error.to_string())?;
    if body.len() < 2 || body.len() > 150 * 1024 * 1024 || body[..2] != *b"MZ" {
        return Err("下载内容不是有效的 Windows 程序".into());
    }
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let folder = current
        .parent()
        .ok_or_else(|| "无法定位当前程序目录".to_owned())?;
    let destination = folder.join(format!("LvwanToolbox-v{version}.exe"));
    if std::fs::canonicalize(&destination).ok() == std::fs::canonicalize(&current).ok() {
        return Err("下载目标与当前运行程序相同，已取消覆盖".into());
    }
    let temporary = folder.join(format!("LvwanToolbox-v{version}.download"));
    fs::write(&temporary, body).map_err(|error| format!("保存更新文件失败：{error}"))?;
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| format!("替换旧更新文件失败：{error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("完成更新文件写入失败：{error}"));
    }
    Ok(destination)
}

fn request_download(host: &str, path: &str) -> io::Result<Vec<u8>> {
    let agent = wide("LvwanMutationToolbox/1.3.4");
    let host = wide(host);
    let verb = wide("GET");
    let path = wide(path);
    let version = wide("HTTP/1.1");
    unsafe {
        let session = Handle::from_raw(
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                null(),
                null(),
                0,
            ),
            "初始化下载失败",
        )?;
        WinHttpSetTimeouts(session.0, 5000, 10000, 15000, 15000);
        let connection = Handle::from_raw(
            WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0),
            "连接下载源失败",
        )?;
        let request = Handle::from_raw(
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                version.as_ptr(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ),
            "创建下载请求失败",
        )?;
        if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return Err(io::Error::other(format!(
                "获取更新文件失败：{}",
                io::Error::last_os_error()
            )));
        }
        let mut status = 0u32;
        let mut status_bytes = size_of::<u32>() as u32;
        let mut header_index = 0;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut status_bytes,
            &mut header_index,
        ) == 0
        {
            return Err(io::Error::other("无法读取下载响应状态"));
        }
        if status != 200 {
            return Err(io::Error::other(format!("下载源返回 HTTP {status}")));
        }
        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(io::Error::other("读取更新文件大小失败"));
            }
            if available == 0 {
                break;
            }
            if body.len() + available as usize > 150 * 1024 * 1024 {
                return Err(io::Error::other("更新文件超过 150 MB"));
            }
            let mut chunk = vec![0; available as usize];
            let mut read = 0u32;
            if WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), available, &mut read) == 0 {
                return Err(io::Error::other("读取更新文件失败"));
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(body)
    }
}

fn request_manifest(mirror: bool) -> io::Result<OnlineManifest> {
    let (host, path) = if mirror {
        ("gh-proxy.com", MIRROR_PATH)
    } else {
        ("raw.githubusercontent.com", API_PATH)
    };
    let agent = wide("LvwanMutationToolbox/1.3.4");
    let host = wide(host);
    let verb = wide("GET");
    let path = wide(path);
    let version = wide("HTTP/1.1");
    let headers = wide("Accept: application/json\r\nUser-Agent: LvwanMutationToolbox/1.3.4\r\n");

    unsafe {
        let session = Handle::from_raw(
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                null(),
                null(),
                0,
            ),
            "初始化网络请求失败",
        )?;
        WinHttpSetTimeouts(session.0, 5000, 7000, 7000, 7000);

        let connection = Handle::from_raw(
            WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0),
            "连接更新源失败",
        )?;
        let request = Handle::from_raw(
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                version.as_ptr(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ),
            "创建更新请求失败",
        )?;
        let sent = WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            null(),
            0,
            0,
            0,
        );
        if sent == 0 || WinHttpReceiveResponse(request.0, null_mut()) == 0 {
            return Err(io::Error::other(format!(
                "读取更新信息失败：{}",
                io::Error::last_os_error()
            )));
        }

        let mut status = 0u32;
        let mut status_bytes = size_of::<u32>() as u32;
        let mut header_index = 0;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut status_bytes,
            &mut header_index,
        ) == 0
        {
            return Err(io::Error::other("无法读取更新源响应状态"));
        }
        if status != 200 {
            return Err(io::Error::other(format!("更新源返回 HTTP {status}")));
        }

        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(io::Error::other(format!(
                    "读取更新内容失败：{}",
                    io::Error::last_os_error()
                )));
            }
            if available == 0 {
                break;
            }
            if body.len() + available as usize > MAX_RESPONSE_BYTES {
                return Err(io::Error::other("更新日志内容超过允许大小"));
            }
            let mut chunk = vec![0; available as usize];
            let mut read = 0u32;
            if WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), available, &mut read) == 0 {
                return Err(io::Error::other(format!(
                    "读取更新内容失败：{}",
                    io::Error::last_os_error()
                )));
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        serde_json::from_slice(&body)
            .map_err(|error| io::Error::other(format!("更新 JSON 格式错误：{error}")))
    }
}

pub fn is_newer(remote: &str, current: &str) -> bool {
    let parse = |value: &str| {
        value
            .trim_start_matches(['v', 'V'])
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
    };
    match (parse(remote), parse(current)) {
        (Ok(remote), Ok(current)) => {
            let max = remote.len().max(current.len());
            (0..max)
                .map(|index| {
                    remote
                        .get(index)
                        .copied()
                        .unwrap_or(0)
                        .cmp(&current.get(index).copied().unwrap_or(0))
                })
                .find(|ordering| !ordering.is_eq())
                .is_some_and(|ordering| ordering.is_gt())
        }
        _ => remote != current,
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
