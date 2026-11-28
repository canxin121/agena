use super::*;

pub struct AgentCatalogue {
    pub name: String,
    pub description: String,
    pub source: String,
    pub entries: Vec<AgentCatalogueEntry>,
}

pub struct AgentCatalogueEntry {
    pub identity: String,
    pub name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub version: Option<String>,
    pub version_count: usize,
    pub registry: Option<String>,
    pub package_source: Option<String>,
}

impl AgentPackageInstaller<'_> {
    pub fn search(
        &self,
        ecosystem: AgentEcosystem,
        cwd: &Path,
        marketplace: Option<&str>,
        query: &str,
    ) -> Result<AgentCatalogue, MarketplaceError> {
        let needle = query.trim().to_lowercase();
        if ecosystem == AgentEcosystem::DeepSeek {
            return self.deepseek_catalogue(marketplace, &needle);
        }
        agena_storage_layout::secure_directory(self.root)?;
        let work = tempfile::Builder::new()
            .prefix(".catalogue-")
            .tempdir_in(self.root)?;
        let source = marketplace
            .map(|source| normalize_location(cwd, source))
            .or_else(|| default_marketplace(ecosystem).map(|(_, source)| source.to_owned()))
            .ok_or_else(|| invalid("a marketplace source is required"))?;
        let catalogue = self.load_catalog(ecosystem, &source, work.path())?;
        let mut entries = Vec::new();
        let plugins = catalogue.document["plugins"]
            .as_array()
            .ok_or_else(|| invalid("marketplace needs a plugins array"))?;
        for plugin in plugins {
            let Some(name) = plugin["name"].as_str().filter(|name| valid_segment(name)) else {
                continue;
            };
            let title = plugin["displayName"].as_str().unwrap_or(name);
            let description = plugin["description"].as_str().unwrap_or_default();
            if format!("{name} {title} {description}")
                .to_lowercase()
                .contains(&needle)
            {
                let version = plugin["version"].as_str().map(str::to_owned);
                entries.push(AgentCatalogueEntry {
                    identity: format!("{name}@{}", catalogue.name),
                    name: title.to_owned(),
                    description: description.to_owned(),
                    homepage: homepage(plugin),
                    version_count: usize::from(version.is_some()),
                    version,
                    registry: Some(source.clone()),
                    package_source: None,
                });
            }
        }
        let location = self.catalog_file(ecosystem, &catalogue.name);
        write_secure_file(
            &location,
            &serde_json::to_vec(&source).map_err(|error| MarketplaceError::index_error(&error))?,
        )?;
        Ok(AgentCatalogue {
            name: catalogue.name,
            description: catalogue.document["description"]
                .as_str()
                .or_else(|| {
                    catalogue
                        .document
                        .pointer("/metadata/description")
                        .and_then(Value::as_str)
                })
                .unwrap_or_default()
                .to_owned(),
            source,
            entries,
        })
    }

    fn deepseek_catalogue(
        &self,
        marketplace: Option<&str>,
        query: &str,
    ) -> Result<AgentCatalogue, MarketplaceError> {
        let Some(source) = marketplace else {
            return self.deepseek_npm_package(query);
        };
        let url = http_url(source)?;
        let document = decode_manifest(&self.fetcher.fetch(url.as_str())?)?;
        if document["format"] != 1 {
            return Err(invalid("the DeepSeek plugin index must use format 1"));
        }
        let name = localized(&document["title"])
            .ok_or_else(|| invalid("DeepSeek plugin index needs a title"))?;
        let plugins = document["plugins"]
            .as_array()
            .ok_or_else(|| invalid("DeepSeek plugin index needs a plugins array"))?;
        let mut entries = Vec::new();
        for plugin in plugins {
            let plugin_name = plugin["name"]
                .as_str()
                .ok_or_else(|| invalid("DeepSeek index plugin needs a name"))?;
            let (package, selector) = npm_selector(plugin_name)?;
            if selector.is_some() || package != plugin_name {
                return Err(invalid(
                    "DeepSeek index plugin names cannot include versions",
                ));
            }
            let title = localized(&plugin["title"]).unwrap_or(plugin_name);
            let description = localized(&plugin["description"]).unwrap_or_default();
            let versions = plugin["versions"]
                .as_array()
                .ok_or_else(|| invalid("DeepSeek index plugin needs versions"))?;
            let version = versions
                .iter()
                .filter_map(|value| {
                    let version = Version::parse(value["version"].as_str()?).ok()?;
                    Some((version, value))
                })
                .max_by(|(left, _), (right, _)| {
                    (left.pre.is_empty(), left).cmp(&(right.pre.is_empty(), right))
                });
            let Some((version, release)) = version else {
                continue;
            };
            if !format!("{plugin_name} {title} {description}")
                .to_lowercase()
                .contains(query)
            {
                continue;
            }
            let release_source = &release["source"];
            let (identity, registry, package_source) = match release_source["type"].as_str() {
                Some("npm") => {
                    let registry = release_source["registry"]
                        .as_str()
                        .map(http_url)
                        .transpose()?
                        .map(|url| url.to_string());
                    (format!("{plugin_name}@{version}"), registry, None)
                }
                Some("git") => {
                    let url = http_url(release_source["url"].as_str().unwrap_or_default())?;
                    let commit = release_source["commit"]
                        .as_str()
                        .filter(|commit| {
                            commit.len() == 40
                                && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                        .ok_or_else(|| {
                            invalid("DeepSeek index Git source needs a full commit ID")
                        })?;
                    if url.fragment().is_some() {
                        return Err(invalid("DeepSeek index Git source cannot have a fragment"));
                    }
                    let mut spec = format!("git+{url}#{commit}");
                    if let Some(path) = release_source["path"].as_str() {
                        if !safe_path(Path::new(path)) {
                            return Err(invalid("invalid package directory in a DeepSeek index"));
                        }
                        spec.push_str("&path:/");
                        spec.push_str(path);
                    }
                    (plugin_name.to_owned(), None, Some(spec))
                }
                _ => return Err(invalid("DeepSeek index release needs an npm or Git source")),
            };
            entries.push(AgentCatalogueEntry {
                identity,
                name: title.to_owned(),
                description: description.to_owned(),
                homepage: homepage(plugin),
                version: Some(version.to_string()),
                version_count: versions.len(),
                registry,
                package_source,
            });
        }
        Ok(AgentCatalogue {
            name: name.to_owned(),
            description: localized(&document["description"])
                .unwrap_or_default()
                .to_owned(),
            source: url.to_string(),
            entries,
        })
    }

    fn deepseek_npm_package(&self, query: &str) -> Result<AgentCatalogue, MarketplaceError> {
        let mut catalogue = AgentCatalogue {
            name: "npm".to_owned(),
            source: "https://registry.npmjs.org/".to_owned(),
            description: String::new(),
            entries: Vec::new(),
        };
        let Ok((name, version)) = npm_selector(query) else {
            return Ok(catalogue);
        };
        let mut url = http_url(&catalogue.source)?;
        url.path_segments_mut()
            .map_err(|_| invalid("invalid npm registry URL"))?
            .pop_if_empty()
            .push(name);
        let index = decode_manifest(&self.fetcher.fetch(url.as_str())?)?;
        let (_, package) = npm_version(&index, version.unwrap_or("latest"))?;
        if package.pointer("/dsh/bundle/patch").is_some() {
            catalogue.entries.push(AgentCatalogueEntry {
                identity: query.to_owned(),
                name: name.to_owned(),
                description: package["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                homepage: homepage(package),
                version: package["version"].as_str().map(str::to_owned),
                version_count: 1,
                registry: None,
                package_source: None,
            });
        }
        Ok(catalogue)
    }
}

fn localized(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value["en"].as_str())
}

fn homepage(document: &Value) -> Option<String> {
    let url = Url::parse(document["homepage"].as_str()?).ok()?;
    matches!(url.scheme(), "https" | "http").then(|| url.to_string())
}
