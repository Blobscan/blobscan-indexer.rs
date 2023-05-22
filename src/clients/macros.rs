#[macro_export]
/// Make a GET request sending and expecting JSON.
/// if JSON deser fails, emit a `WARN` level tracing event
macro_rules! json_get {
    ($client:expr, $url:expr, $expected:ty) => {
        json_get!($client, $url, $expected, "")
    };
    ($client:expr, $url:expr, $expected:ty, $auth_token:expr) => {{
        let url = $url.clone();
        tracing::debug!(url = url.as_str(), method = "GET", "Dispatching API request");

        let mut req = $client.get($url);

        if !$auth_token.is_empty() {
          req = req.bearer_auth($auth_token);
        }

        let resp = req.send().await?;
        let status = resp.status();

        if status.as_u16() == 404 {
          return Ok(None)
        };

        let text = resp.text().await?;
        let result: Result<crate::clients::common::ClientResponse<$expected>, _> = serde_json::from_str(&text);

        match result {
            Err(e) => {
                tracing::warn!(
                    method = "GET",
                    url = %url,
                    response = text.as_str(),
                    "Unexpected response from server"
                );
                println!("here");
                println!("{e}");
                Err(e.into())
            },
            Ok(response) => {
              response.into_client_result()
            }
        }
    }};
}

#[macro_export]
/// Make a PUT request sending JSON.
/// if JSON deser fails, emit a `WARN` level tracing event
macro_rules! json_put {
    ($client:expr, $url:expr, $auth_token:expr, $body:expr) => {{
        let url = $url.clone();
        let body = format!("{:?}", $body);

        tracing::debug!(url = url.as_str(), method = "PUT", body = body, "Dispatching API client request");


        let resp = $client
            .put($url)
            .bearer_auth($auth_token)
            .json($body)
            .send()
            .await?;

        let text = resp.text().await?;
        let result: crate::clients::common::ClientResponse<_> = text.parse()?;

        if result.is_err() {
            tracing::warn!(
                method = "PUT",
                url = %url,
                body = body,
                response = text.as_str(),
                "Unexpected response from server"
            );
        }

        result.into_client_result()
    }};
}
