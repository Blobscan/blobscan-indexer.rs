#[macro_export]
/// Make a GET request sending and expecting JSON with retry using exponential backoff.
/// if JSON deser fails, emit a `WARN` level tracing event
macro_rules! json_get {
    ($client:expr, $url:expr, $expected:ty, $exp_backoff:expr) => {
        json_get!($client, $url, $expected, "", $exp_backoff)
    };
    ($client:expr, $url:expr, $expected:ty, $auth_token:expr, $exp_backoff: expr) => {{
        let url = $url.clone();

        tracing::trace!(method = "GET", url = url.as_str(), "Dispatching API request");

        let mut req = $client.get($url);

        if !$auth_token.is_empty() {
          req = req.bearer_auth($auth_token);
        }

        let resp = match backoff::future::retry_notify(
            $exp_backoff,
            || {
                let req = req.try_clone().unwrap();

                async move { req.send().await.map_err(|err| err.into()) }
            },
                |error, duration: std::time::Duration| {
                    let duration = duration.as_secs();

                    tracing::warn!(
                        method = "GET",
                        url = %url,
                        ?error,
                        "Failed to send request. Retrying in {duration} seconds…"
                    );
                },
            )
        .await {
            Ok(resp) => resp,
            Err(error) => {
                tracing::warn!(
                    method = "GET",
                    url = %url,
                    ?error,
                    "Failed to send request. All retries failed"
                );

                return Err(error.into())
            }
        };

        let status = resp.status();

        if status.as_u16() == 404 {
          return Ok(None)
        };

        let text = resp.text().await?;
        let result: Result<$crate::clients::common::ClientResponse<$expected>, _> = serde_json::from_str(&text);

        match result {
            Err(e) => {
                tracing::warn!(
                    method = "GET",
                    url = %url,
                    response = text.as_str(),
                    "Unexpected response from server"
                );

                Err(e.into())
            },
            Ok(response) => {
              response.into_client_result()
            }
        }
    }};
}

#[macro_export]
/// Make a PUT request sending JSON with retry using exponential backoff.
/// if JSON deser fails, emit a `WARN` level tracing event
macro_rules! json_put {
    ($client:expr, $url:expr, $auth_token:expr, $body:expr, $exp_backoff:expr) => {
        json_put!($client, $url, (), $auth_token, $body, $exp_backoff)
    };
    ($client:expr, $url:expr, $expected:ty, $auth_token:expr, $body:expr, $exp_backoff:expr) => {{
        let url = $url.clone();
        let body = format!("{:?}", $body);

        tracing::trace!(method = "PUT", url = url.as_str(), body, "Dispatching API client request");

        let resp = match backoff::future::retry_notify(
            $exp_backoff,
            || {
                let req = $client
                    .put($url.clone())
                    .bearer_auth($auth_token.clone())
                    .json($body);

                async move { req.send().await.map_err(|err| err.into()) }
            },
            |error, duration: std::time::Duration| {
                let duration = duration.as_secs();

                tracing::warn!(
                    method = "PUT",
                    url = %url,
                    ?error,
                    "Failed to send request. Retrying in {duration} seconds…"
                );
            },
        )
        .await {
            Ok(resp) => resp,
            Err(error) => {
                tracing::warn!(
                    method = "PUT",
                    url = %url,
                    ?error,
                    "Failed to send request. All retries failed"
                );

                return Err(error.into())
            }
        };

        let text = resp.text().await?;
        let result: $crate::clients::common::ClientResponse<$expected> = text.parse()?;

        if result.is_err() {
            tracing::warn!(
                method = "PUT",
                url = %url,
                body,
                response = text.as_str(),
                "Unexpected response from server"
            );
        }

        result.into_client_result()
    }};
}
