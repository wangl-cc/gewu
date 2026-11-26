use std::{
    fs::File,
    io::{BufWriter, Write as _},
    path::Path,
    ptr,
    sync::{Mutex, mpsc},
};

use objc2::{AnyThread, DefinedClass, define_class, rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSObject, NSObjectProtocol, NSString, NSURL, NSURLRequest,
    NSURLRequestCachePolicy, NSURLResponse, NSURLSession, NSURLSessionConfiguration,
    NSURLSessionDataDelegate, NSURLSessionDataTask, NSURLSessionDelegate,
    NSURLSessionResponseDisposition, NSURLSessionTask, NSURLSessionTaskDelegate,
};

use crate::release::Result;

enum Sink {
    Memory(Vec<u8>),
    File(BufWriter<File>),
}

struct DownloadState {
    max_bytes: u64,
    received: u64,
    sink: Sink,
    error: Option<String>,
    sender: Option<mpsc::Sender<std::result::Result<Vec<u8>, String>>>,
}

impl DownloadState {
    fn fail(&mut self, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(message.into());
        }
    }
}

struct DelegateIvars {
    state: Mutex<DownloadState>,
}

define_class!(
    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - `DownloadDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[ivars = DelegateIvars]
    struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}

    unsafe impl NSURLSessionDelegate for DownloadDelegate {}

    unsafe impl NSURLSessionTaskDelegate for DownloadDelegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn will_redirect(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            _response: &NSHTTPURLResponse,
            request: &NSURLRequest,
            completion_handler: &block2::DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            let is_https = request
                .URL()
                .and_then(|url| url.scheme())
                .is_some_and(|scheme| scheme.to_string().eq_ignore_ascii_case("https"));
            if is_https {
                let next: *const NSURLRequest = request;
                completion_handler.call((next.cast_mut(),));
            } else {
                if let Ok(mut state) = self.ivars().state.lock() {
                    state.fail("Blocked non-HTTPS redirect.");
                }
                completion_handler.call((ptr::null_mut(),));
            }
        }

        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            let Ok(mut state) = self.ivars().state.lock() else {
                return;
            };
            let Some(sender) = state.sender.take() else {
                return;
            };
            let outcome = if let Some(message) = state.error.take() {
                Err(message)
            } else if let Some(ns_error) = error {
                Err(ns_error.localizedDescription().to_string())
            } else {
                match &mut state.sink {
                    Sink::Memory(bytes) => Ok(std::mem::take(bytes)),
                    Sink::File(writer) => writer
                        .flush()
                        .map(|()| Vec::new())
                        .map_err(|err| err.to_string()),
                }
            };
            let _ = sender.send(outcome);
        }
    }

    unsafe impl NSURLSessionDataDelegate for DownloadDelegate {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        fn did_receive_response(
            &self,
            _session: &NSURLSession,
            _data_task: &NSURLSessionDataTask,
            response: &NSURLResponse,
            completion_handler: &block2::DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            let mut rejection = None;
            if let Some(http) = response.downcast_ref::<NSHTTPURLResponse>() {
                let status = http.statusCode();
                if !(200..=299).contains(&status) {
                    rejection = Some(format!("HTTP request failed with status {status}."));
                } else if let Ok(state) = self.ivars().state.lock() {
                    let expected = http.expectedContentLength();
                    if expected > 0 && expected as u64 > state.max_bytes {
                        rejection = Some("The download exceeds the permitted size.".to_owned());
                    }
                }
            } else {
                rejection = Some("Unexpected non-HTTP response.".to_owned());
            }

            if let Some(message) = rejection {
                if let Ok(mut state) = self.ivars().state.lock() {
                    state.fail(message);
                }
                completion_handler.call((NSURLSessionResponseDisposition::Cancel,));
            } else {
                completion_handler.call((NSURLSessionResponseDisposition::Allow,));
            }
        }

        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn did_receive_data(
            &self,
            _session: &NSURLSession,
            data_task: &NSURLSessionDataTask,
            data: &NSData,
        ) {
            let chunk = data.to_vec();
            let cancel = {
                let Ok(mut state) = self.ivars().state.lock() else {
                    return;
                };
                match state.received.checked_add(chunk.len() as u64) {
                    Some(total) if total <= state.max_bytes => {
                        state.received = total;
                        match &mut state.sink {
                            Sink::Memory(bytes) => {
                                bytes.extend_from_slice(&chunk);
                                false
                            }
                            Sink::File(writer) => match writer.write_all(&chunk) {
                                Ok(()) => false,
                                Err(err) => {
                                    state.fail(err.to_string());
                                    true
                                }
                            },
                        }
                    }
                    _ => {
                        state.fail("The download exceeds the permitted size.");
                        true
                    }
                }
            };
            if cancel {
                data_task.cancel();
            }
        }
    }
);

impl DownloadDelegate {
    #[expect(unsafe_code, reason = "initializes an NSObject subclass with ivars")]
    fn new(state: DownloadState) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars {
            state: Mutex::new(state),
        });
        // SAFETY: `this` is freshly allocated with initialized ivars and `NSObject::init` completes
        // initialization.
        unsafe { objc2::msg_send![super(this), init] }
    }
}

pub(crate) fn get_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>> {
    fetch(url, Sink::Memory(Vec::new()), max_bytes)
}

pub(crate) fn download_to_file(url: &str, dest: &Path, max_bytes: u64) -> Result<()> {
    let file = File::create(dest)?;
    fetch(url, Sink::File(BufWriter::new(file)), max_bytes)?;
    Ok(())
}

#[expect(unsafe_code, reason = "creates an NSURLSession with a typed delegate")]
fn fetch(url: &str, sink: Sink, max_bytes: u64) -> Result<Vec<u8>> {
    if !url.starts_with("https://") {
        return Err("Only HTTPS URLs are permitted for updates.".into());
    }
    let ns_url = NSURL::URLWithString(&NSString::from_str(url)).ok_or("Invalid update URL.")?;
    let request = NSURLRequest::requestWithURL(&ns_url);

    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    config.setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
    config.setTimeoutIntervalForRequest(30.0);
    config.setTimeoutIntervalForResource(300.0);

    let (sender, receiver) = mpsc::channel();
    let delegate = DownloadDelegate::new(DownloadState {
        max_bytes,
        received: 0,
        sink,
        error: None,
        sender: Some(sender),
    });

    // SAFETY: `delegate` implements `NSURLSessionDelegate` and lives independently in `session`;
    // passing `None` lets Foundation create a serial operation queue.
    let session = unsafe {
        NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
            &config,
            Some(ProtocolObject::from_ref(&*delegate)),
            None,
        )
    };
    let task = session.dataTaskWithRequest(&request);
    task.resume();

    let result = receiver
        .recv()
        .map_err(|_| "Update download terminated unexpectedly.")?;
    if result.is_ok() {
        session.finishTasksAndInvalidate();
    } else {
        session.invalidateAndCancel();
    }
    result.map_err(Into::into)
}
