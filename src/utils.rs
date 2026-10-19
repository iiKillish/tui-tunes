use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

type Job = Box<dyn FnOnce() + Send>;

/// Jobs run one at a time, in the order they were given, on a thread of their own (started on
/// first use): for slow work, like requests to Spotify, that mustn't hold up the interface but
/// must happen in order.
pub struct InOrder {
    name: &'static str,
    jobs: OnceLock<Mutex<mpsc::Sender<Job>>>,
}

impl InOrder {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            jobs: OnceLock::new(),
        }
    }

    /// Run `job` after the jobs given before it.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        let jobs = self.jobs.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name(self.name.into())
                .spawn(move || rx.into_iter().for_each(|job| job()))
                .expect("could not start a background thread");
            Mutex::new(tx)
        });
        let _ = jobs.lock().unwrap().send(Box::new(job));
    }
}

/// Returns a human readable String of milliseconds in the HH:MM:SS format.
pub fn ms_to_hms(duration: u32) -> String {
    let mut formated_time = String::new();

    let total_seconds = duration / 1000;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;

    if hours > 0 {
        formated_time.push_str(&format!("{hours}:{minutes:02}:"));
    } else {
        formated_time.push_str(&format!("{minutes}:"));
    }
    formated_time.push_str(&format!("{seconds:02}"));

    formated_time
}

/// Parse a duration like `1h 30m`, `1m30s`, `90s`, `1.5 min` or `500ms`: numbers, each followed
/// by a unit (`ms`, `s`, `m` or `h`, or longer names like `sec`, `minutes`, `hours`).
///
/// Written for ttunes instead of the `parse_duration` crate, which is unmaintained and can be
/// made to use unbounded CPU and memory by a large exponent (GHSA-qpgv-g792-wh6x). Exponents
/// aren't accepted here, and a total too large for a [Duration] is an error.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let mut seconds = 0f64;
    let mut rest = text.trim();
    if rest.is_empty() {
        return Err("expected a duration such as 1m 30s".into());
    }
    while !rest.is_empty() {
        let number_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let number: f64 = rest[..number_len]
            .parse()
            .map_err(|_| format!("expected a number at \"{rest}\""))?;
        rest = rest[number_len..].trim_start();
        let unit_len = rest
            .find(|c: char| !c.is_alphabetic())
            .unwrap_or(rest.len());
        let unit = match rest[..unit_len].to_lowercase().as_str() {
            "ms" | "msec" | "msecs" | "millisecond" | "milliseconds" => 0.001,
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "" => return Err(format!("{number} needs a unit, such as s or m")),
            other => return Err(format!("unknown unit \"{other}\"")),
        };
        seconds += number * unit;
        rest = rest[unit_len..].trim_start();
    }
    Duration::try_from_secs_f64(seconds).map_err(|_| "the duration is too long".into())
}

pub fn cache_path_for_url(url: String) -> std::path::PathBuf {
    let mut path = crate::config::cache_path("covers");
    path.push(url.split('/').next_back().unwrap());
    path
}

pub fn download(url: String, path: std::path::PathBuf) -> Result<(), std::io::Error> {
    let mut resp = reqwest::blocking::get(url).map_err(std::io::Error::other)?;

    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut file = std::fs::File::create(path)?;

    std::io::copy(&mut resp, &mut file)?;
    Ok(())
}

/// Create the application specific runtime directory and return the path to it.
///
/// If the directory already exists and has the correct permissions, this function just returns the
/// existing directory. The contents stored in this directory are not necessarily persisted across
/// reboots. Stored files should be small since they could reside in memory (like on a tmpfs mount).
#[cfg(unix)]
pub fn create_runtime_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    use std::{
        fs::{self, Permissions},
        os::unix::prelude::PermissionsExt,
    };

    let user_runtime_directory = user_runtime_directory().ok_or("no runtime directory found")?;

    let creation_result = fs::create_dir(&user_runtime_directory);

    if creation_result.is_ok()
        || matches!(
            creation_result.as_ref().unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        )
    {
        // Needed when created inside a world readable directory, to prevent unauthorized access.
        // Doesn't hurt otherwise.
        fs::set_permissions(&user_runtime_directory, Permissions::from_mode(0o700))?;

        Ok(user_runtime_directory)
    } else {
        #[allow(clippy::unnecessary_unwrap)]
        Err(Box::new(creation_result.unwrap_err()))
    }
}

/// Lowercase `text` and take the accents off its letters, for searches where "beyonce" finds
/// "Beyoncé".
pub fn search_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect()
}

/// Hide the password in a proxy URL like `http://user:password@host:8080`, for logging it.
///
/// Anything that doesn't look like it holds a password is returned unchanged.
pub fn redact_proxy_password(proxy: &str) -> String {
    let (scheme, rest) = match proxy.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), proxy),
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if let Some(at) = authority.rfind('@')
        && let Some((user, _)) = authority[..at].split_once(':')
    {
        return format!("{scheme}{user}:***{}", &rest[at..]);
    }
    proxy.to_owned()
}

/// Return the path to the current user's runtime directory, or None if it couldn't be found.
/// This function does not guarantee correct ownership or permissions of the directory.
#[cfg(unix)]
pub fn user_runtime_directory() -> Option<PathBuf> {
    let linux_runtime_directory =
        PathBuf::from(format!("/run/user/{}/", unsafe { libc::getuid() }));
    let unix_runtime_directory = PathBuf::from("/tmp/");

    if let Some(xdg_runtime_directory) = xdg_runtime_directory() {
        Some(xdg_runtime_directory.join("ttunes"))
    } else if cfg!(target_os = "linux") && linux_runtime_directory.exists() {
        Some(linux_runtime_directory.join("ttunes"))
    } else if unix_runtime_directory.exists() {
        Some(unix_runtime_directory.join(format!("ttunes-{}", unsafe { libc::getuid() })))
    } else {
        None
    }
}

#[cfg(unix)]
fn xdg_runtime_directory() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR").ok().map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{parse_duration, redact_proxy_password, search_text};
    use std::time::Duration;

    #[test]
    fn durations_combine_numbers_and_units() {
        let secs = |text| parse_duration(text).map(|d| d.as_secs_f64());
        assert_eq!(secs("90s"), Ok(90.0));
        assert_eq!(secs("1h 30m"), Ok(5400.0));
        assert_eq!(secs("1m30s"), Ok(90.0));
        assert_eq!(secs(" 1.5 Minutes "), Ok(90.0));
        assert_eq!(secs("2 hours 1 sec"), Ok(7201.0));
        assert_eq!(parse_duration("500ms"), Ok(Duration::from_millis(500)));
    }

    #[test]
    fn durations_need_numbers_with_units() {
        for text in ["", "90", "m", "1x", "1.2.3s", "-1s", "1e9s", "s1"] {
            assert!(parse_duration(text).is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn huge_durations_are_an_error_not_a_hang() {
        assert!(parse_duration(&format!("{}h", "9".repeat(400))).is_err());
        assert!(parse_duration("1e1000000000s").is_err());
    }

    #[test]
    fn proxy_passwords_are_hidden() {
        assert_eq!(
            redact_proxy_password("http://me:secret@proxy:8080/path"),
            "http://me:***@proxy:8080/path"
        );
        assert_eq!(
            redact_proxy_password("socks5h://me:p@ss@proxy"),
            "socks5h://me:***@proxy"
        );
        assert_eq!(redact_proxy_password("me:secret@proxy"), "me:***@proxy");
        assert_eq!(redact_proxy_password("http://me@proxy"), "http://me@proxy");
        assert_eq!(
            redact_proxy_password("http://proxy:8080/a:b@c"),
            "http://proxy:8080/a:b@c"
        );
    }

    #[test]
    fn search_text_ignores_accents_and_case() {
        assert_eq!(search_text("Tití Me Preguntó"), "titi me pregunto");
        assert_eq!(search_text("CAFÉ"), "cafe");
        assert_eq!(search_text("Beyonce\u{301}"), "beyonce");
        assert_eq!(search_text("Sigur Rós ÅÖ"), "sigur ros ao");
    }
}
