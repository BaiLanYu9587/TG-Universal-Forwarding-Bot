use super::model::ForwardTask;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::{error, warn};

enum DispatchMessage {
    Task(ForwardTask),
    Shutdown,
}

pub struct Dispatcher {
    task_sender: tokio::sync::mpsc::UnboundedSender<DispatchMessage>,
    task_receiver: Option<tokio::sync::mpsc::UnboundedReceiver<DispatchMessage>>,
    workers: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    inflight: Arc<AtomicUsize>,
    idle_notify: Arc<Notify>,
    closed: Arc<AtomicBool>,
}

impl Dispatcher {
    pub fn new(worker_count: usize) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(worker_count));

        Self {
            task_sender: sender,
            task_receiver: Some(receiver),
            workers: Arc::new(std::sync::Mutex::new(Vec::new())),
            semaphore,
            inflight: Arc::new(AtomicUsize::new(0)),
            idle_notify: Arc::new(Notify::new()),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start<F, Fut>(&mut self, handler: F)
    where
        F: Fn(ForwardTask) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let mut receiver = self.task_receiver.take().expect("dispatcher 已启动");
        let semaphore = self.semaphore.clone();
        let inflight = self.inflight.clone();
        let idle_notify = self.idle_notify.clone();

        let worker = tokio::spawn(async move {
            while let Some(message) = receiver.recv().await {
                match message {
                    DispatchMessage::Task(task) => {
                        let handler = handler.clone();
                        let semaphore = semaphore.clone();
                        let inflight = inflight.clone();
                        let idle_notify = idle_notify.clone();

                        inflight.fetch_add(1, Ordering::Relaxed);
                        tokio::spawn(async move {
                            let _permit = semaphore.acquire().await.unwrap();

                            // 将 handler 包装在独立的 task 中，以便捕获 panic
                            let handler = handler.clone();
                            let task = tokio::spawn(async move {
                                handler(task).await;
                            });

                            // 等待任务完成并检查是否 panic
                            match task.await {
                                Ok(_) => {}
                                Err(e) => {
                                    if e.is_panic() {
                                        error!("Worker 处理任务时 panic，任务已丢弃");
                                        // 尝试提取 panic 信息
                                        let panic_msg = e.into_panic();
                                        if let Some(msg) = panic_msg.downcast_ref::<&str>() {
                                            error!("Panic 消息: {}", msg);
                                        } else if let Some(msg) = panic_msg.downcast_ref::<String>()
                                        {
                                            error!("Panic 消息: {}", msg);
                                        }
                                    } else {
                                        error!("Worker 任务被取消: {}", e);
                                    }
                                }
                            }

                            if inflight.fetch_sub(1, Ordering::AcqRel) == 1 {
                                idle_notify.notify_waiters();
                            }
                        });
                    }
                    DispatchMessage::Shutdown => {
                        break;
                    }
                }
            }
        });

        if let Ok(mut workers) = self.workers.lock() {
            workers.push(worker);
        }
    }

    pub fn send(&self, task: ForwardTask) {
        self.dispatch(task);
    }

    pub fn dispatch(&self, task: ForwardTask) {
        if self.closed.load(Ordering::Relaxed) {
            warn!("dispatcher 已关闭，丢弃任务");
            return;
        }
        if self.task_sender.send(DispatchMessage::Task(task)).is_err() {
            warn!("dispatcher 已停止，丢弃任务");
        }
    }

    pub async fn shutdown_graceful(&self, timeout: std::time::Duration) -> bool {
        self.closed.store(true, Ordering::Release);
        let _ = self.task_sender.send(DispatchMessage::Shutdown);

        let handles = if let Ok(mut workers) = self.workers.lock() {
            std::mem::take(&mut *workers)
        } else {
            Vec::new()
        };
        for handle in handles {
            let _ = handle.await;
        }

        tokio::time::timeout(timeout, self.wait_idle())
            .await
            .is_ok()
    }

    pub async fn wait_idle(&self) {
        self.wait_idle_internal().await;
    }

    async fn wait_idle_internal(&self) {
        loop {
            if self.inflight.load(Ordering::Acquire) == 0 {
                break;
            }
            self.idle_notify.notified().await;
        }
    }
}

impl Clone for Dispatcher {
    fn clone(&self) -> Self {
        Self {
            task_sender: self.task_sender.clone(),
            task_receiver: None,
            workers: self.workers.clone(),
            semaphore: self.semaphore.clone(),
            inflight: self.inflight.clone(),
            idle_notify: self.idle_notify.clone(),
            closed: self.closed.clone(),
        }
    }
}
