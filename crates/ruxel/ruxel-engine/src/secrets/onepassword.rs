//! The 1Password field lookup of the resolver: the structured
//! `onePassword` selector and the `op://` reference both read one field of
//! one item through it, with the CLI's own selector grammar mirrored.

use super::{Resolver, cached, select_field};

impl Resolver {
    pub(super) fn op_field(
        &self,
        vault: &str,
        item: &str,
        field: &str,
        section: Option<&str>,
    ) -> Result<String, String> {
        // A section and field reach `op item get --fields` as one selector.
        // The CLI parses `,` as a list separator, `=` as `label=`/`type=`
        // selectors and `.` as the section separator, so a legal component
        // like "API,key" or "API.Password" would fetch a different selector
        // than the author named. Authoring refuses the same characters
        // (Core's `onePassword`); this mirrors it, because a configuration
        // can name a selector directly.
        if let Some(section) = section
            && (section.contains(',') || section.contains('='))
        {
            return Err(format!(
                "1Password section \"{section}\" may not contain a comma or an equals sign: \
                 `op item get --fields` reads them as selectors, so it would fetch unintended fields"
            ));
        }
        if let Some(section) = section
            && section.contains('.')
        {
            return Err(format!(
                "1Password section \"{section}\" may not contain a dot: \
                 `op item get --fields` reads a dot as the section separator, so it would fetch a different selector than the author named"
            ));
        }
        if field.contains(',') || field.contains('=') {
            return Err(format!(
                "1Password field \"{field}\" may not contain a comma or an equals sign: \
                 `op item get --fields` reads them as selectors, so it would fetch unintended fields"
            ));
        }
        if field.contains('.') {
            return Err(format!(
                "1Password field \"{field}\" may not contain a dot: \
                 `op item get --fields` reads a dot as the section separator, so it would fetch a different selector than the author named"
            ));
        }
        // The fetch is field- and section-scoped, so the cache is too: the
        // same label in two sections is two distinct reads and never shares
        // a fetched document.
        let cell = self
            .items
            .lock()
            .map_err(|_| "lookup cache unavailable")?
            .entry((
                vault.to_owned(),
                item.to_owned(),
                section.map(str::to_owned),
                field.to_owned(),
            ))
            .or_default()
            .clone();
        let document = cached(&cell, || {
            self.backend
                .onepassword_fields(vault, item, section, field)
                .map_err(|_| "secret lookup failed".to_owned())
        })?;
        select_field(&document, item, field, section)
    }
}
