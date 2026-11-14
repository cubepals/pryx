// SPDX-License-Identifier: FSL-1.1-ALv2
//! Backend file responses preserve the existing node API while streaming through its owned
//! workload guard. Upload admission checks each incoming chunk; no mutation is replayed,
//! no error body or private agent header becomes a public node response.
use super::*;
use crate::{
    manager::files::{DataAccess, DataLocation, file_error},
    runtime::files::Operation,
};
use http_body_util::BodyExt;

fn status(status: StatusCode, expected: StatusCode) -> Result<(), Response> {
    if status == expected {
        Ok(())
    } else if status == StatusCode::NOT_FOUND {
        Err(NodeError::FileNotFound.into_response())
    } else if status == StatusCode::UNPROCESSABLE_ENTITY {
        Err(file_error(std::io::Error::from(std::io::ErrorKind::InvalidInput)).into_response())
    } else {
        Err(NodeError::RuntimeUnavailable(format!("File service refused {status}")).into_response())
    }
}
fn failure(problem: crate::runtime::RuntimeError) -> Response {
    NodeError::from(problem).into_response()
}
async fn upload(
    manager: Arc<Manager>,
    body: Body,
    sender: tokio::sync::mpsc::Sender<Result<hyper::body::Frame<bytes::Bytes>, std::io::Error>>,
) -> Result<(), Response> {
    let mut input = body.into_data_stream();
    use futures_util::TryStreamExt;
    while let Some(chunk) = input
        .try_next()
        .await
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "file_interrupted", "File upload interrupted", None))?
    {
        manager.file_room(chunk.len() as u64).map_err(IntoResponse::into_response)?;
        sender.send(Ok(hyper::body::Frame::data(chunk))).await.map_err(|_| {
            NodeError::RuntimeUnavailable("File service stopped receiving upload".into()).into_response()
        })?;
    }
    Ok(())
}
pub(super) async fn proxy(
    access: DataAccess,
    operation: Operation,
    path: &str,
    body: Option<Body>,
    manager: Arc<Manager>,
) -> Result<Response, Response> {
    let DataLocation::Remote(service) = access.location() else {
        return Err(NodeError::Internal("File service access is not remote".into()).into_response());
    };
    let writing = matches!(operation, Operation::Write);
    let reading = matches!(operation, Operation::Read);
    let expected = if writing { StatusCode::NO_CONTENT } else { StatusCode::OK };
    let response = if let Some(body) = body {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|frame| (frame, receiver))
        });
        let body_stream = BodyExt::boxed(http_body_util::StreamBody::new(stream));
        let sending = async {
            let response = service.request(operation, path, body_stream).await.map_err(failure)?;
            status(response.status(), expected)?;
            Ok::<_, Response>(response)
        };
        tokio::try_join!(sending, upload(manager, body, sender)).map(|(response, ())| response)
    } else {
        match service.request(operation, path, crate::http_client::empty()).await {
            Ok(response) => status(response.status(), expected).map(|()| response),
            Err(problem) => Err(failure(problem)),
        }
    };
    let response = match response {
        Ok(response) => response,
        Err(response) => {
            access.finish().await.map_err(IntoResponse::into_response)?;
            return Err(response);
        }
    };
    if writing {
        drop(response);
        access.finish().await.map_err(IntoResponse::into_response)?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let length = response.headers().get(header::CONTENT_LENGTH).cloned();
    let stream = response.into_body().into_data_stream().map(move |chunk| {
        let _held = &access;
        chunk
    });
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(if reading { "application/octet-stream" } else { "application/json" }),
    );
    if let Some(length) = length {
        response.headers_mut().insert(header::CONTENT_LENGTH, length);
    }
    Ok(response)
}
