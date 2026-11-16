// SPDX-License-Identifier: FSL-1.1-ALv2
//! Docker Engine event observation. Events are hints to inspect a container, translated only
//! here from Docker replies. Container creation and inspection stay in their own modules.
use super::*;

impl DockerRuntime {
    pub(super) fn event_stream(&self, label_filters: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        let mut filters = filters(label_filters);
        filters.insert("type".into(), vec!["container".into()]);
        let options = EventsOptionsBuilder::new().filters(&filters).build();
        let docker = match self.docker() {
            Ok(docker) => docker,
            Err(e) => return futures_util::stream::iter([Err(e)]).boxed(),
        };
        let own = self.oci_runtime;
        docker
            .events(Some(options))
            .filter(move |item| std::future::ready(gvisor::owns_event(own, item)))
            .map(|item| {
                let message = item.map_err(map_err)?;
                let actor = message.actor.unwrap_or_default();
                let at = message
                    .time_nano
                    .and_then(|ns| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ns)).ok())
                    .unwrap_or_else(OffsetDateTime::now_utc);
                Ok(RuntimeEvent {
                    container_id: actor.id.unwrap_or_default(),
                    action: message.action.unwrap_or_default(),
                    exit_code: actor.attributes.as_ref().and_then(|a| a.get("exitCode")).and_then(|c| c.parse().ok()),
                    at,
                })
            })
            .boxed()
    }
}
