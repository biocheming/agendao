use agendao_tool_core::{drain_piped_output, MAX_CAPTURED_OUTPUT_BYTES, MAX_CAPTURED_STREAM_BYTES};
use tokio::io::{AsyncWriteExt, DuplexStream};

async fn write_and_close(mut stream: DuplexStream, bytes: Vec<u8>) {
    stream.write_all(&bytes).await.unwrap();
    stream.shutdown().await.unwrap();
}

#[tokio::test]
async fn eof_on_stdout_does_not_stop_stderr_drain() {
    let (stdout_writer, stdout_reader) = tokio::io::duplex(64);
    let (stderr_writer, stderr_reader) = tokio::io::duplex(64);
    let stdout_task = tokio::spawn(write_and_close(stdout_writer, b"stdout-first\n".to_vec()));
    let stderr_task = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        write_and_close(stderr_writer, b"stderr-after-stdout-eof\n".to_vec()).await;
    });

    let output = drain_piped_output(stdout_reader, stderr_reader)
        .await
        .unwrap();
    stdout_task.await.unwrap();
    stderr_task.await.unwrap();
    assert_eq!(output.stdout, b"stdout-first\n");
    assert_eq!(output.stderr, b"stderr-after-stdout-eof\n");
}

#[tokio::test]
async fn giant_unterminated_stream_is_bounded_but_fully_drained() {
    let (stdout_writer, stdout_reader) = tokio::io::duplex(16 * 1024);
    let (stderr_writer, stderr_reader) = tokio::io::duplex(16 * 1024);
    let huge = vec![b'x'; MAX_CAPTURED_OUTPUT_BYTES * 3];
    let writer = tokio::spawn(write_and_close(stdout_writer, huge));
    drop(stderr_writer);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        drain_piped_output(stdout_reader, stderr_reader),
    )
    .await
    .expect("collector must keep reading instead of blocking the writer")
    .unwrap();
    writer.await.unwrap();
    assert_eq!(output.stdout.len(), MAX_CAPTURED_STREAM_BYTES);
    assert!(output.stdout_truncated);
    assert!(output.truncated());
}
