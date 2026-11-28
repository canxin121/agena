//! Resolve Claude marketplace dependencies before publishing any configuration.

use std::collections::{BTreeMap, BTreeSet};

use semver::VersionReq;

use super::*;

const MANIFEST: &str = ".claude-plugin/plugin.json";
const MAX_DEPENDENCIES: usize = 128;
const MAX_DEPTH: usize = 32;
const MAX_SELECTIONS: usize = MAX_DEPENDENCIES * 2;

#[derive(Clone)]
pub struct ClaudeDependencyVersion(Vec<VersionReq>);

impl ClaudeDependencyVersion {
    pub fn parse(value: &str) -> Result<Self, MarketplaceError> {
        if value.trim().is_empty() || value.len() > 256 {
            return Err(invalid("invalid Claude dependency version range"));
        }
        let ranges = value
            .split("||")
            .map(|range| {
                let range = range.trim();
                let comparators = if let Some((lower, upper)) = range.split_once(" - ") {
                    format!(">={}, <={}", lower.trim(), upper.trim())
                } else {
                    let separated = range.replace(',', " ");
                    let mut terms = separated.split_whitespace();
                    let mut comparators = Vec::new();
                    while let Some(term) = terms.next() {
                        comparators.push(match term {
                            "=" | ">" | "<" | ">=" | "<=" | "~" | "^" => format!(
                                "{term}{}",
                                terms
                                    .next()
                                    .ok_or_else(|| invalid("incomplete version comparator"))?,
                            ),
                            _ => {
                                let term = term.strip_prefix('v').unwrap_or(term);
                                if Version::parse(term).is_ok() {
                                    format!("={term}")
                                } else if term.split('.').all(|part| part.parse::<u64>().is_ok()) {
                                    format!("{term}.*")
                                } else {
                                    term.replace(['x', 'X'], "*")
                                }
                            }
                        });
                    }
                    if comparators.is_empty() {
                        return Err(invalid("empty version comparator"));
                    }
                    comparators.join(", ")
                };
                VersionReq::parse(&comparators)
                    .map_err(|_| invalid(format!("invalid Claude dependency range {value:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(ranges))
    }

    pub fn matches(&self, version: &str) -> bool {
        Version::parse(version.trim_start_matches('v'))
            .is_ok_and(|version| self.matches_version(&version))
    }

    fn matches_version(&self, version: &Version) -> bool {
        self.0.iter().any(|range| range.matches(version))
    }
}

pub struct InstalledClaudePackage {
    pub path: PathBuf,
    pub enabled: bool,
}

#[derive(Default)]
pub struct ClaudeDependencyPlan {
    pub packages: Vec<PreparedAgentPackage>,
    pub bindings: Bindings,
    pub reused: BTreeMap<String, PathBuf>,
}

struct Dependency {
    identity: String,
    version: Option<(String, ClaudeDependencyVersion)>,
}

fn manifest(root: &Path) -> Result<Map<String, Value>, MarketplaceError> {
    if root.join(MANIFEST).is_file() {
        read_object(&root.join(MANIFEST))
    } else {
        read_object(&root.join("plugin.json"))
    }
}

fn declarations(
    document: &Map<String, Value>,
    marketplace: &str,
) -> Result<Vec<Dependency>, MarketplaceError> {
    let Some(items) = document.get("dependencies") else {
        return Ok(Vec::new());
    };
    let array = items
        .as_array()
        .ok_or_else(|| invalid("Claude dependencies must be an array"))?;
    if array.len() > MAX_DEPENDENCIES {
        return Err(invalid("too many Claude plugin dependencies"));
    }
    let mut dependencies = Vec::new();
    let mut seen = BTreeSet::new();
    for item in array {
        let field = |field: &str| {
            item.get(field)
                .map(|value| {
                    value.as_str().ok_or_else(|| {
                        invalid(format!("Claude dependency {field} must be a string"))
                    })
                })
                .transpose()
        };
        let (name, market, version) = match item {
            Value::String(selector) => {
                let (name, market) = split_selector(selector)?;
                (name, market, None)
            }
            Value::Object(_) => (
                field("name")?.ok_or_else(|| invalid("Claude dependency needs a name"))?,
                field("marketplace")?,
                field("version")?,
            ),
            _ => return Err(invalid("invalid Claude plugin dependency")),
        };
        let market = market.unwrap_or(marketplace);
        if !valid_segment(name) || !valid_segment(market) {
            return Err(invalid(
                "invalid Claude dependency plugin or marketplace name",
            ));
        }
        if !seen.insert((name, market)) {
            return Err(invalid(format!(
                "duplicate Claude dependency {name}@{market}"
            )));
        }
        dependencies.push(Dependency {
            identity: format!("{name}@{market}"),
            version: version
                .map(|value| {
                    ClaudeDependencyVersion::parse(value).map(|range| (value.to_owned(), range))
                })
                .transpose()?,
        });
    }
    Ok(dependencies)
}

type InstalledLookup<'a> =
    dyn Fn(&str) -> Result<Option<InstalledClaudePackage>, MarketplaceError> + 'a;

impl AgentPackageInstaller<'_> {
    pub fn prepare_claude_dependencies(
        &self,
        primary: &PreparedAgentPackage,
        installed: &InstalledLookup<'_>,
    ) -> Result<ClaudeDependencyPlan, MarketplaceError> {
        let Some(catalog) = primary.catalog_snapshot.as_ref() else {
            if !declarations(&manifest(&primary.path)?, "direct")?.is_empty() {
                return Err(invalid(
                    "install this Claude plugin from its marketplace to resolve its declared dependencies",
                ));
            }
            return Ok(ClaudeDependencyPlan::default());
        };
        let cross_marketplaces: BTreeSet<String> = catalog
            .document
            .get("allowCrossMarketplaceDependenciesOn")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect();
        let mut resolver = Resolver {
            installer: self,
            primary_catalog: catalog,
            installed,
            cross_marketplaces,
            workspace: tempfile::Builder::new()
                .prefix(".dependencies-")
                .tempdir_in(self.root)?,
            catalogues: BTreeMap::new(),
            releases: BTreeMap::new(),
            selected: BTreeMap::new(),
            selections: 0,
        };
        let primary = Node::load(&primary.identity, &primary.path)?;
        let bindings = resolver.solve(&primary).map_err(SearchError::into_error)?;
        let mut plan = ClaudeDependencyPlan {
            bindings,
            ..Default::default()
        };
        for (identity, selection) in resolver.selected {
            match selection.package {
                SelectedPackage::Prepared(package) => plan.packages.push(package),
                SelectedPackage::Installed(path) => {
                    plan.reused.insert(identity, path);
                }
            }
        }
        Ok(plan)
    }

    fn prepare_dependency(
        &self,
        catalogue: &Catalog,
        entry: &Map<String, Value>,
        identity: &str,
        release: Option<(&ReleaseSource, &Release)>,
    ) -> Result<PreparedAgentPackage, MarketplaceError> {
        let temp = tempfile::Builder::new()
            .prefix(".dependency-")
            .tempdir_in(self.root)?;
        let work = temp.path().join("source");
        fs::create_dir(&work)?;
        let root = match release {
            Some((source, release)) => {
                let root = checkout(&source.url, Some(&release.tag), None, &work.join("release"))?;
                match &source.subdir {
                    Some(path) => directory_under(&root, path)?,
                    None => root,
                }
            }
            None => self.entry_root(catalogue, entry, identity, None, &work)?,
        };
        let path = temp.path().join("package");
        copy_package(&root, &path)?;
        materialize_manifest(AgentEcosystem::Claude, &path, entry, self.fetcher)?;
        if let Some((_, release)) = release {
            let mut document = manifest(&path)?;
            document.insert("version".into(), Value::String(release.version.to_string()));
            let bytes = serde_json::to_vec_pretty(&document)
                .map_err(|error| MarketplaceError::index_error(&error))?;
            write_secure_file(&path.join(MANIFEST), &bytes)?;
        }
        let key = hex::encode(Sha256::digest(
            format!("{identity}\n{}", catalogue.source).as_bytes(),
        ));
        Ok(PreparedAgentPackage {
            identity: identity.to_owned(),
            path,
            source: AgentPluginSource {
                ecosystem: AgentEcosystem::Claude,
                selector: identity.to_owned(),
                registry: Some(catalogue.source.clone()),
                package_source: None,
            },
            diagnostics: Vec::new(),
            temp,
            catalog_snapshot: None,
            origin: self.root.join("packages").join("claude").join(key),
            catalog: Some((
                self.catalog_file(AgentEcosystem::Claude, &catalogue.name),
                catalogue.source.clone(),
            )),
        })
    }
}

type Bindings = BTreeMap<String, BTreeMap<String, Option<String>>>;

struct Node {
    identity: String,
    version: Option<String>,
    dependencies: Vec<Dependency>,
}

impl Node {
    fn load(identity: &str, path: &Path) -> Result<Self, MarketplaceError> {
        let (name, marketplace) = split_selector(identity)?;
        let marketplace = marketplace.ok_or_else(|| invalid("missing dependency marketplace"))?;
        let document = manifest(path)?;
        if document.get("name").and_then(Value::as_str) != Some(name) {
            return Err(invalid(format!(
                "plugin manifest name does not match {identity}"
            )));
        }
        Ok(Self {
            identity: identity.to_owned(),
            version: document
                .get("version")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            dependencies: declarations(&document, marketplace)?,
        })
    }
}

enum SelectedPackage {
    Prepared(PreparedAgentPackage),
    Installed(PathBuf),
}

struct Selection {
    node: Node,
    package: SelectedPackage,
}

enum SearchError {
    Invalid(MarketplaceError),
    Version {
        identity: String,
        error: MarketplaceError,
    },
    Limit,
}

impl From<MarketplaceError> for SearchError {
    fn from(error: MarketplaceError) -> Self {
        Self::Invalid(error)
    }
}

impl SearchError {
    fn into_error(self) -> MarketplaceError {
        match self {
            Self::Invalid(error) | Self::Version { error, .. } => error,
            Self::Limit => invalid("Claude plugin dependency graph exceeds its resolution limit"),
        }
    }
}

struct Requirement {
    consumer: String,
    version: String,
    range: ClaudeDependencyVersion,
}

impl Requirement {
    fn validate(&self, identity: &str, version: Option<&str>) -> Result<(), SearchError> {
        if version.is_some_and(|version| self.range.matches(version)) {
            return Ok(());
        }
        Err(SearchError::Version {
            identity: identity.to_owned(),
            error: invalid(format!(
                "{} requires {identity} {}; the selected package has version {}",
                self.consumer,
                self.version,
                version.unwrap_or("unspecified")
            )),
        })
    }
}

struct Pending {
    depth: usize,
    requirements: Vec<Requirement>,
}

#[derive(Default)]
struct Graph {
    bindings: Bindings,
    pending: BTreeMap<String, Pending>,
}

impl Graph {
    fn visit(
        &mut self,
        node: &Node,
        selected: &BTreeMap<String, Selection>,
        stack: &mut Vec<String>,
    ) -> Result<(), SearchError> {
        if self.bindings.contains_key(&node.identity) {
            return Ok(());
        }
        if self.bindings.len() + self.pending.len() + stack.len() >= MAX_DEPENDENCIES
            || stack.len() >= MAX_DEPTH
        {
            return Err(SearchError::Limit);
        }
        stack.push(node.identity.clone());
        let mut requires = BTreeMap::new();
        for dependency in &node.dependencies {
            let identity = &dependency.identity;
            if stack.contains(identity) {
                return Err(invalid(format!(
                    "Claude plugin dependency cycle from {} to {identity}",
                    node.identity
                ))
                .into());
            }
            let required = dependency
                .version
                .as_ref()
                .map(|(version, range)| Requirement {
                    consumer: node.identity.clone(),
                    version: version.clone(),
                    range: range.clone(),
                });
            if let Some(selection) = selected.get(identity) {
                if let Some(requirement) = required {
                    requirement.validate(identity, selection.node.version.as_deref())?;
                }
                self.visit(&selection.node, selected, stack)?;
            } else {
                let depth = stack.len();
                let pending = self
                    .pending
                    .entry(identity.clone())
                    .or_insert_with(|| Pending {
                        depth,
                        requirements: Vec::new(),
                    });
                pending.depth = pending.depth.min(depth);
                pending.requirements.extend(required);
                if self.bindings.len() + self.pending.len() + stack.len() > MAX_DEPENDENCIES {
                    return Err(SearchError::Limit);
                }
            }
            requires.insert(
                identity.clone(),
                dependency
                    .version
                    .as_ref()
                    .map(|(version, _)| version.clone()),
            );
        }
        stack.pop();
        self.bindings.insert(node.identity.clone(), requires);
        Ok(())
    }
}

struct Resolver<'a, 'i> {
    installer: &'a AgentPackageInstaller<'i>,
    primary_catalog: &'a Catalog,
    installed: &'a InstalledLookup<'a>,
    cross_marketplaces: BTreeSet<String>,
    workspace: TempDir,
    catalogues: BTreeMap<String, Catalog>,
    releases: BTreeMap<String, Option<ReleaseSource>>,
    selected: BTreeMap<String, Selection>,
    selections: usize,
}

impl Resolver<'_, '_> {
    fn solve(&mut self, primary: &Node) -> Result<Bindings, SearchError> {
        let mut graph = Graph::default();
        graph.visit(primary, &self.selected, &mut Vec::new())?;
        let Some((identity, pending)) =
            graph.pending.into_iter().min_by_key(|(_, item)| item.depth)
        else {
            return Ok(graph.bindings);
        };
        let requirements = pending.requirements;
        if let Some(installed) = (self.installed)(&identity)? {
            if !installed.enabled {
                return Err(invalid(format!("Claude dependency {identity} is disabled")).into());
            }
            return self.select(
                primary,
                &identity,
                SelectedPackage::Installed(installed.path),
                &requirements,
            );
        }

        let (name, market) = split_selector(&identity)?;
        let market = market.expect("dependency identity includes a marketplace");
        self.load_catalog(market, &identity)?;
        let entry = self.catalog(market).entry(name)?;
        let source = entry.get("source");
        let kind = source
            .and_then(|source| source.get("source"))
            .and_then(Value::as_str);
        if kind == Some("command")
            || source.is_some_and(|source| source.get("headersHelper").is_some())
            || entry.contains_key("headersHelper")
        {
            return Err(invalid(format!("install dependency {identity} explicitly first")).into());
        }

        // Unconstrained dependencies track the marketplace. An edge discovered
        // farther down the graph can still require a tagged release of this package.
        let mut mismatch = None;
        if requirements.is_empty() {
            match self.select_current(primary, &identity, market, name, &requirements) {
                Err(error)
                    if matches!(&error, SearchError::Version { identity: conflict, .. }
                    if *conflict == identity) =>
                {
                    mismatch = Some(error)
                }
                result => return result,
            }
        }
        if !self.releases.contains_key(&identity) {
            let catalogue = self.catalog(market);
            let release_source = ReleaseSource::load(catalogue, catalogue.entry(name)?, name)?;
            self.releases.insert(identity.clone(), release_source);
        }
        let source = self.releases[&identity].as_ref();
        let mut had_matching_tag = false;
        let mut last_error = mismatch;
        if let Some(source) = source {
            let releases = source
                .releases
                .iter()
                .filter(|release| {
                    requirements
                        .iter()
                        .all(|required| required.range.matches_version(&release.version))
                })
                .cloned()
                .collect::<Vec<_>>();
            for release in releases {
                had_matching_tag = true;
                let source = self.releases[&identity]
                    .as_ref()
                    .expect("release source loaded");
                let catalogue = self.catalog(market);
                let package = self.installer.prepare_dependency(
                    catalogue,
                    catalogue.entry(name)?,
                    &identity,
                    Some((source, &release)),
                )?;
                match self.select(
                    primary,
                    &identity,
                    SelectedPackage::Prepared(package),
                    &requirements,
                ) {
                    Ok(bindings) => return Ok(bindings),
                    Err(SearchError::Limit) => return Err(SearchError::Limit),
                    Err(error) => {
                        last_error = Some(error);
                    }
                }
            }
        }
        if had_matching_tag || requirements.is_empty() {
            return Err(last_error.expect("an attempted dependency selection failed"));
        }
        if self.releases[&identity]
            .as_ref()
            .is_some_and(|source| source.external)
        {
            let constraints = requirements
                .iter()
                .map(|requirement| format!("{} from {}", requirement.version, requirement.consumer))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(invalid(format!(
                "dependency {identity} has no Git release tag satisfying {constraints}"
            ))
            .into());
        }
        self.select_current(primary, &identity, market, name, &requirements)
    }

    fn select_current(
        &mut self,
        primary: &Node,
        identity: &str,
        market: &str,
        name: &str,
        requirements: &[Requirement],
    ) -> Result<Bindings, SearchError> {
        let catalogue = self.catalog(market);
        let package =
            self.installer
                .prepare_dependency(catalogue, catalogue.entry(name)?, identity, None)?;
        self.select(
            primary,
            identity,
            SelectedPackage::Prepared(package),
            requirements,
        )
    }

    fn select(
        &mut self,
        primary: &Node,
        identity: &str,
        package: SelectedPackage,
        requirements: &[Requirement],
    ) -> Result<Bindings, SearchError> {
        self.selections += 1;
        if self.selections > MAX_SELECTIONS {
            return Err(SearchError::Limit);
        }
        let path = match &package {
            SelectedPackage::Prepared(package) => &package.path,
            SelectedPackage::Installed(path) => path,
        };
        let node = Node::load(identity, path)?;
        for requirement in requirements {
            requirement.validate(identity, node.version.as_deref())?;
        }
        self.selected
            .insert(identity.to_owned(), Selection { node, package });
        let result = self.solve(primary);
        if result.is_err() {
            self.selected.remove(identity);
        }
        result
    }

    fn catalog(&self, market: &str) -> &Catalog {
        if market == self.primary_catalog.name {
            self.primary_catalog
        } else {
            &self.catalogues[market]
        }
    }

    fn load_catalog(&mut self, market: &str, identity: &str) -> Result<(), MarketplaceError> {
        if market == self.primary_catalog.name || self.catalogues.contains_key(market) {
            return Ok(());
        }
        if !self.cross_marketplaces.contains(market) {
            return Err(invalid(format!(
                "Claude dependency {identity} is outside the root marketplace's allowCrossMarketplaceDependenciesOn"
            )));
        }
        let source = self
            .installer
            .catalog_source(AgentEcosystem::Claude, market)?;
        let work = self.workspace.path().join(market);
        fs::create_dir(&work)?;
        let catalogue = self
            .installer
            .load_catalog(AgentEcosystem::Claude, &source, &work)?;
        if catalogue.name != market {
            return Err(invalid(format!(
                "registered marketplace source does not match {market:?}"
            )));
        }
        self.catalogues.insert(market.to_owned(), catalogue);
        Ok(())
    }
}

struct ReleaseSource {
    url: String,
    subdir: Option<String>,
    external: bool,
    releases: Vec<Release>,
}

#[derive(Clone)]
struct Release {
    tag: String,
    version: Version,
}

impl ReleaseSource {
    fn load(
        catalogue: &Catalog,
        entry: &Map<String, Value>,
        name: &str,
    ) -> Result<Option<Self>, MarketplaceError> {
        let Some(source) = entry.get("source") else {
            return Ok(None);
        };
        let kind = source.get("source").and_then(Value::as_str);
        if source.as_str().is_some() || kind == Some("local") {
            if matches!(&catalogue.files, CatalogFiles::Manifest) {
                return Ok(None);
            }
            let raw_path = source
                .as_str()
                .or_else(|| source.get("path").and_then(Value::as_str))
                .ok_or_else(|| invalid("missing local dependency path"))?;
            let subdir = match catalogue.plugin_root_for(raw_path) {
                Some(prefix) => format!("{prefix}/{raw_path}"),
                _ => raw_path.to_owned(),
            };
            // For a remote catalogue use the original source so shallow
            // marketplace checkouts cannot hide older plugin release tags.
            let url = if let CatalogFiles::Directory(root) = &catalogue.files
                && Path::new(&catalogue.source).exists()
            {
                if git2::Repository::open(root).is_err() {
                    return Ok(None);
                }
                Url::from_directory_path(root)
                    .map_err(|()| invalid("invalid local marketplace directory"))?
                    .to_string()
            } else {
                git_location(&catalogue.source)?.0
            };
            return Self::from_git(url, name, Some(subdir), false).map(Some);
        }
        if matches!(kind, Some("github" | "url" | "git-subdir"))
            && source.get("sha").is_none()
            && source.get("ref").is_none()
        {
            let location = source
                .get(if kind == Some("github") {
                    "repo"
                } else {
                    "url"
                })
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("missing dependency repository"))?;
            let (url, reference) = git_location(location)?;
            if reference.is_none() {
                let subdir = source
                    .get("path")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                return Self::from_git(url, name, subdir, true).map(Some);
            }
        }
        Ok(None)
    }

    fn from_git(
        url: String,
        name: &str,
        subdir: Option<String>,
        external: bool,
    ) -> Result<Self, MarketplaceError> {
        let mut remote = git2::Remote::create_detached(url.as_str())
            .map_err(|error| invalid(format!("could not open dependency repository: {error}")))?;
        let connection = remote
            .connect_auth(git2::Direction::Fetch, None, Some(git_proxy_options()))
            .map_err(|error| invalid(format!("could not read dependency release tags: {error}")))?;
        let prefix = format!("refs/tags/{name}--v");
        let mut releases = connection
            .list()
            .map_err(|error| invalid(format!("could not list dependency release tags: {error}")))?
            .iter()
            .filter_map(|head| {
                let raw = head.name().strip_prefix(&prefix)?;
                let version = Version::parse(raw).ok()?;
                Some(Release {
                    version,
                    tag: head.name().to_owned(),
                })
            })
            .collect::<Vec<_>>();
        releases.sort_unstable_by(|left, right| right.version.cmp(&left.version));
        Ok(Self {
            url,
            subdir,
            external,
            releases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Offline;

    impl HttpFetcher for Offline {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
            panic!("unexpected fetch of {url}");
        }
    }

    fn installer(root: &Path) -> AgentPackageInstaller<'_> {
        AgentPackageInstaller {
            root,
            fetcher: &Offline,
            command: &|_, _, _| panic!("unexpected install command"),
        }
    }

    fn write_json(path: &Path, value: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn plugin_path(market: &Path, name: &str) -> PathBuf {
        market.join("packages").join(name)
    }

    fn plugin(market: &Path, name: &str, version: &str, requires: Value) {
        write_json(
            &plugin_path(market, name).join(MANIFEST),
            &json!({"name": name, "version": version, "dependencies": requires}),
        );
    }

    fn market(root: &Path, name: &str, plugins: &[&str], allowed: &[&str]) {
        let entries = plugins
            .iter()
            .map(|name| json!({"name": name, "source": format!("./packages/{name}")}))
            .collect::<Vec<_>>();
        write_json(
            &root.join(".claude-plugin/marketplace.json"),
            &json!({
                "name": name,
                "plugins": entries,
                "allowCrossMarketplaceDependenciesOn": allowed,
            }),
        );
    }

    fn prepare(
        installer: &AgentPackageInstaller<'_>,
        root: &Path,
        name: &str,
    ) -> PreparedAgentPackage {
        installer
            .prepare(AgentPackageRequest {
                ecosystem: AgentEcosystem::Claude,
                selector: name,
                marketplace: Some(root.to_str().unwrap()),
                package_source: None,
                cwd: root,
                accept_command: None,
            })
            .unwrap()
    }

    fn commit(repo: &git2::Repository, tag: Option<&str>) {
        let signature = git2::Signature::now("fixture", "fixture@example.test").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().map(|head| head.peel_to_commit().unwrap());
        let parents = parent.iter().collect::<Vec<_>>();
        let id = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "fixture",
                &tree,
                &parents,
            )
            .unwrap();
        if let Some(tag) = tag {
            let target = repo.find_object(id, None).unwrap();
            repo.tag(tag, &target, &signature, "fixture", false)
                .unwrap();
        }
    }

    #[test]
    fn shared_dependency_uses_highest_tag_matching_all_edges() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let installer = installer(&cache);
        let catalog = temp.path().join("market");
        let repository = git2::Repository::init(&catalog).unwrap();
        market(&catalog, "team", &["main", "left", "right", "shared"], &[]);
        plugin(&catalog, "main", "1.0.0", json!(["left", "right"]));
        plugin(
            &catalog,
            "left",
            "1.0.0",
            json!([{"name":"shared","version":"^2.0"}]),
        );
        plugin(
            &catalog,
            "right",
            "1.0.0",
            json!([{"name":"shared","version":"~2.1"}]),
        );
        for (number, contents) in [("2.0.0", "old"), ("2.1.3", "selected"), ("2.2.0", "new")] {
            plugin(&catalog, "shared", "1.9.0", json!([]));
            fs::write(plugin_path(&catalog, "shared").join("marker"), contents).unwrap();
            commit(&repository, Some(&format!("shared--v{number}")));
        }

        let primary = prepare(&installer, &catalog, "main");
        let plan = installer
            .prepare_claude_dependencies(&primary, &|_| Ok(None))
            .unwrap();
        assert_eq!(plan.packages.len(), 3);
        assert_eq!(
            plan.bindings["left@team"]["shared@team"].as_deref(),
            Some("^2.0")
        );
        assert_eq!(
            plan.bindings["right@team"]["shared@team"].as_deref(),
            Some("~2.1")
        );
        let shared = plan
            .packages
            .iter()
            .find(|package| package.identity == "shared@team")
            .unwrap();
        assert_eq!(manifest(&shared.path).unwrap()["version"], "2.1.3");
        assert_eq!(
            fs::read_to_string(shared.path.join("marker")).unwrap(),
            "selected"
        );

        plugin(
            &catalog,
            "right",
            "1.0.0",
            json!([{"name":"shared","version":"^3.0"}]),
        );
        let primary = prepare(&installer, &catalog, "main");
        let error = installer
            .prepare_claude_dependencies(&primary, &|_| Ok(None))
            .err()
            .unwrap();
        assert!(error.to_string().contains("shared@team"), "{error}");
    }

    #[test]
    fn tries_another_tag_when_the_latest_conflicts_transitively() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let installer = installer(&cache);
        let catalog = temp.path().join("market");
        let repository = git2::Repository::init(&catalog).unwrap();
        market(&catalog, "team", &["main", "branch", "shared"], &[]);
        plugin(
            &catalog,
            "main",
            "1.0.0",
            json!([
                {"name":"branch","version":"^1.0"},
                {"name":"shared","version":"^1.0"}
            ]),
        );
        plugin(&catalog, "shared", "1.2.0", json!([]));
        plugin(&catalog, "branch", "1.0.0", json!(["shared"]));
        commit(&repository, Some("branch--v1.0.0"));
        plugin(
            &catalog,
            "branch",
            "1.1.0",
            json!([{"name":"shared","version":"^2.0"}]),
        );
        commit(&repository, Some("branch--v1.1.0"));

        let primary = prepare(&installer, &catalog, "main");
        let plan = installer
            .prepare_claude_dependencies(&primary, &|_| Ok(None))
            .unwrap();
        assert_eq!(plan.bindings["branch@team"]["shared@team"], None);
        let branch = plan
            .packages
            .iter()
            .find(|package| package.identity == "branch@team")
            .unwrap();
        assert_eq!(manifest(&branch.path).unwrap()["version"], "1.0.0");
    }

    #[test]
    fn root_allowlist_applies_to_transitive_dependencies_and_installed_copies() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let installer = installer(&cache);
        let team = temp.path().join("team");
        let shared = temp.path().join("shared");
        market(&team, "team", &["main", "branch"], &[]);
        market(&shared, "shared", &["helper"], &["team"]);
        plugin(&shared, "helper", "1.2.3", json!([]));
        prepare(&installer, &shared, "helper")
            .publish()
            .unwrap()
            .save_catalog()
            .unwrap();
        plugin(&team, "main", "1.0.0", json!(["branch"]));
        plugin(
            &team,
            "branch",
            "1.0.0",
            json!([{"name":"helper","marketplace":"shared","version":"^1.0"}]),
        );
        let primary = prepare(&installer, &team, "main");
        let error = installer
            .prepare_claude_dependencies(&primary, &|_| Ok(None))
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("allowCrossMarketplaceDependenciesOn"),
            "{error}"
        );

        let installed_path = plugin_path(&shared, "helper");
        let existing = |enabled| {
            let path = installed_path.clone();
            move |identity: &str| {
                Ok(
                    (identity == "helper@shared").then(|| InstalledClaudePackage {
                        path: path.clone(),
                        enabled,
                    }),
                )
            }
        };
        let reused = installer
            .prepare_claude_dependencies(&primary, &existing(true))
            .unwrap();
        assert_eq!(reused.reused["helper@shared"], installed_path);
        assert_eq!(reused.packages.len(), 1);
        let error = installer
            .prepare_claude_dependencies(&primary, &existing(false))
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("helper@shared is disabled"),
            "{error}"
        );

        market(&team, "team", &["main", "branch"], &["shared"]);
        let primary = prepare(&installer, &team, "main");
        let plan = installer
            .prepare_claude_dependencies(&primary, &|_| Ok(None))
            .unwrap();
        assert_eq!(plan.packages.len(), 2);
        assert!(plan.bindings.contains_key("helper@shared"));

        plugin(&shared, "helper", "1.2.3", json!(["main@team"]));
        let error = installer
            .prepare_claude_dependencies(&primary, &|_| Ok(None))
            .err()
            .unwrap();
        assert!(error.to_string().contains("dependency cycle"), "{error}");
    }
}
