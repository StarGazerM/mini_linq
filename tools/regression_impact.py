"""Conservative regression selection from Git changes and Cargo dependency graphs.

A missing proof of independence selects a target. Metadata is read only: no builds,
network access, lockfile updates, or temporary Git worktrees are needed.
"""
import json
from pathlib import Path, PurePosixPath
import subprocess
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


ROOT_KINDS = {"test", "example", "lib"}
MANIFEST_KINDS = {"manifest-test", "manifest-target-test", "manifest-feature-test", "manifest-run", "oracle", "oracle-test"}


def _documentation(path):
    parts = PurePosixPath(path).parts
    name = parts[-1].lower()
    changelog = name == "changelog" or (
        name.startswith(("changelog.", "changelog-"))
        and PurePosixPath(name).suffix in {".txt", ".rst", ".old", ".adoc", ".text", ".markdown"}
    )
    return parts[0] == "doc" or name.endswith(".md") or changelog


class Impact:
    """Select inventory rows; an empty reasons list is a proof-based skip.

    ``until=None`` compares ``since`` with tracked working-tree contents and adds
    nonignored untracked paths. ``until`` instead compares two exact Git trees.
    Historical lockfiles supplement today's metadata with historical local
    packages, so removing a dependency since that range cannot hide an impact.
    """

    def __init__(self, root, rows, since, until=None):
        # Import lazily: the runner imports this helper and owns scenario contexts.
        from test_regressions import NON_CARGO_SCENARIOS, SCENARIO_CARGO

        self.root = Path(root).resolve()
        self.rows = [tuple(row) for row in rows]
        self.since = since
        self.until = until
        self.errors = []
        self._contexts = {}
        self._row_contexts = {}
        self._non_cargo = NON_CARGO_SCENARIOS
        self._owners = {}
        self._manifests = {}
        self._historical_manifests = {}
        self._name_roots = {}
        self._historical_name_roots = {}
        self.changed_files = self._changes()
        self._code_changes = [path for path in self.changed_files if not _documentation(path)]
        self._global = []
        if not self._code_changes:
            return
        self._inventory()
        for row in self.rows:
            _, kind, _, source = row
            if kind in ROOT_KINDS:
                contexts = [("Cargo.toml", ".")]
            elif kind in MANIFEST_KINDS:
                contexts = [(source, ".")]
            elif kind == "scenario":
                contexts = SCENARIO_CARGO.get(source, [])
            else:
                contexts = []
            self._row_contexts[row] = [tuple(context) for context in contexts]
            for context in self._row_contexts[row]:
                if context not in self._contexts:
                    self._contexts[context] = self._metadata(*context)
        self._changed_roots = {}
        for path in self._code_changes:
            parts = PurePosixPath(path).parts
            if parts[-1] in {"Cargo.toml", "Cargo.lock"} or ".cargo" in parts:
                self._global.append(f"dependency/configuration changed: {path}")
                continue
            owner = self._owner(path)
            if owner is None:
                self._global.append(f"unmapped code/configuration/data changed: {path}")
            else:
                self._changed_roots.setdefault(owner, []).append(path)

    def _run(self, command):
        return subprocess.run(command, cwd=self.root, capture_output=True, text=True)

    def _git(self, *args):
        result = self._run(["git", *args])
        if result.returncode:
            raise ValueError(result.stderr.strip() or f"git {' '.join(args)} failed")
        return result.stdout

    def _changes(self):
        try:
            # Disabling rename detection reports both old and new names, including
            # staged renames subsequently edited in the working tree.
            args = ["diff", "--name-only", "--no-renames", "-z", self.since]
            if self.until is not None:
                args.append(self.until)
            changed = self._git(*args, "--").split("\0")
            if self.until is None:
                changed += self._git("ls-files", "--others", "--exclude-standard", "-z").split("\0")
            return sorted(set(filter(None, changed)))
        except (OSError, ValueError) as error:
            self.errors.append(f"change detection unavailable: {error}")
            return []

    def _inventory(self):
        try:
            paths = self._git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0")
            self._load_manifests(paths, None, self._manifests, self._name_roots)
            if self.until is not None:
                historical = self._git("ls-tree", "-r", "--name-only", "-z", self.until).split("\0")
                self._load_manifests(historical, self.until, self._historical_manifests,
                                     self._historical_name_roots)
        except (OSError, ValueError) as error:
            self.errors.append(f"manifest inventory unavailable: {error}")

    def _text(self, path, revision=None):
        if revision is not None:
            return self._git("show", f"{revision}:{path}")
        return (self.root / path).read_text()

    def _load_manifests(self, paths, revision, manifests, names):
        for path in sorted(set(paths)):
            if PurePosixPath(path).name != "Cargo.toml":
                continue
            try:
                data = tomllib.loads(self._text(path, revision))
            except (OSError, ValueError):
                # A deleted or unparseable manifest is never evidence of an
                # unaffected crate. Its changes are global, and unknown files
                # below it are global too.
                continue
            directory = str(PurePosixPath(path).parent)
            manifests[directory] = data
            package = data.get("package", {})
            if isinstance(package.get("name"), str):
                names.setdefault(package["name"], set()).add(directory)
                self._owners[directory] = True
            else:
                self._owners.setdefault(directory, False)

    def _relative(self, path):
        return Path(path).resolve().relative_to(self.root).as_posix()

    def _lock_graph(self, workspace, historical=False):
        revision = self.until if historical else None
        names = self._historical_name_roots if historical else self._name_roots
        path = str(PurePosixPath(workspace) / "Cargo.lock")
        data = tomllib.loads(self._text(path, revision))
        roots = set()
        packages = data.get("package")
        if not isinstance(packages, list) or not packages:
            raise ValueError(f"missing package inventory in {path}")
        for package in packages:
            if "source" not in package:
                name = package["name"]
                if name not in names:
                    raise ValueError(f"local lockfile package {name} has no known manifest")
                roots.update(names[name])
        return {"roots": roots, "packages": packages}

    def _historical_workspace(self, manifest):
        directory = PurePosixPath(manifest).parent
        for candidate in (directory, *directory.parents):
            data = self._historical_manifests.get(str(candidate))
            if data is not None and "workspace" in data:
                return str(candidate)
        # A standalone package need not declare [workspace].
        if str(directory) in self._historical_manifests:
            return str(directory)
        raise ValueError(f"historical manifest unavailable: {manifest}")

    def _historical_scope(self, context, starts):
        """Walk only target dependencies, not sibling lockfile entries."""
        by_name = {}
        for package in context["historical_lock"]["packages"]:
            by_name.setdefault(package["name"], []).append(package)
        seen = set()
        pending = list(starts)
        roots = set()
        while pending:
            name = pending.pop()
            if name in seen:
                continue
            if name not in by_name:
                return None
            seen.add(name)
            # Lock dependency strings may include a version and source. Taking
            # every matching version is conservative without including siblings
            # whose package names are not reachable from this target.
            for package in by_name[name]:
                if "source" not in package:
                    roots.update(self._historical_name_roots[name])
                pending.extend(dependency.split()[0] for dependency in package.get("dependencies", []))
        return roots

    def _metadata(self, manifest, cwd):
        context = {"manifest": manifest, "cwd": cwd, "error": None,
                   "packages": {}, "edges": {}, "members": set(), "lock_roots": set(),
                   "defaults": set(), "targets": {}, "historical_lock": None}
        try:
            result = subprocess.run(
                ["cargo", "metadata", "--locked", "--offline", "--all-features",
                 "--format-version", "1", "--manifest-path", str(self.root / manifest)],
                cwd=self.root / cwd, capture_output=True, text=True,
            )
            if result.returncode:
                raise ValueError(result.stderr.strip() or "cargo metadata failed")
            metadata = json.loads(result.stdout)
            packages = {package["id"]: package for package in metadata["packages"]}
            context["workspace"] = self._relative(metadata["workspace_root"])
            context["members"] = set(metadata["workspace_members"])
            if not context["members"] or not context["members"] <= packages.keys():
                raise ValueError("metadata has incomplete workspace members")
            defaults = metadata.get("workspace_default_members")
            root = (metadata.get("resolve") or {}).get("root")
            directory = str(PurePosixPath(manifest).parent)
            if root and directory != context["workspace"]:
                # --manifest-path pointing at a member operates on that exact
                # package, not the workspace root's default-member selection.
                context["defaults"] = {root}
            elif defaults is not None:
                context["defaults"] = set(defaults)
            else:
                # Older Cargo metadata lacks this field. Explicit workspace
                # defaults cannot safely be guessed.
                data = self._manifests.get(directory, {})
                if "default-members" not in data.get("workspace", {}):
                    context["defaults"] = {root} if root else set(context["members"])
            if not context["defaults"] <= context["members"]:
                raise ValueError("metadata has invalid default members")
            for identifier, package in packages.items():
                if package.get("source") is None:
                    directory = self._relative(Path(package["manifest_path"]).parent)
                    context["packages"][identifier] = (package["name"], directory)
                    self._owners[directory] = True
                    context["targets"][identifier] = package.get("targets")
            nodes = metadata["resolve"]["nodes"]
            for node in nodes:
                # No kind or platform filtering: dev, build, optional and
                # target-specific edges must all participate in the closure.
                edges = set(node["dependencies"])
                edges.update(dependency["pkg"] for dependency in node.get("deps", []))
                context["edges"][node["id"]] = edges
            if not packages.keys() <= context["edges"].keys():
                raise ValueError("metadata has incomplete dependency nodes")
            if any(not edges <= packages.keys() for edges in context["edges"].values()):
                raise ValueError("metadata has unresolved dependency edges")
            if self.until is not None:
                workspace = self._historical_workspace(manifest)
                context["historical_lock"] = self._lock_graph(workspace, historical=True)
                context["lock_roots"] = context["historical_lock"]["roots"]
                directory = str(PurePosixPath(manifest).parent)
                historical = self._historical_manifests.get(directory, {})
                current_defaults = self._manifests.get(directory, {}).get("workspace", {}).get("default-members")
                historical_defaults = historical.get("workspace", {}).get("default-members")
                if current_defaults != historical_defaults:
                    raise ValueError(f"historical default-member selection changed: {manifest}")
                if "package" in historical and directory not in context["lock_roots"]:
                    # An excluded standalone package or explicit workspace
                    # override must not inherit an unrelated ancestor's lock.
                    raise ValueError(f"historical lockfile does not cover {manifest}")
        except (OSError, ValueError, KeyError, TypeError) as error:
            context["error"] = str(error)
            # Keep local lockfile identities as useful diagnostics, never as
            # proof that an unresolved metadata context is safe to skip.
            try:
                directory = PurePosixPath(manifest).parent
                for candidate in (directory, *directory.parents):
                    if "workspace" in self._manifests.get(str(candidate), {}):
                        context["lock_roots"] = self._lock_graph(str(candidate))["roots"]
                        break
            except (OSError, ValueError, KeyError, TypeError):
                pass
        return context

    def _owner(self, path):
        candidate = PurePosixPath(path).parent
        for directory in (candidate, *candidate.parents):
            name = str(directory)
            if name not in self._owners:
                continue
            if not self._owners[name]:
                return None
            # The root package does not own arbitrary repository tooling/data.
            if name == "." and PurePosixPath(path).parts[0] not in {
                "src", "tests", "examples", "benches", "build.rs",
            }:
                return None
            return name
        return None

    @staticmethod
    def _target_owners(context, candidates, kind, target):
        owners = set()
        for identifier in candidates:
            targets = context["targets"].get(identifier)
            if targets is None:
                return None
            if any((target is None or item["name"] == target) and kind in item["kind"] for item in targets):
                owners.add(identifier)
        return owners

    def _scope(self, row, context):
        packages = context["packages"]
        if row[1] in ROOT_KINDS:
            starts = {identifier for identifier in context["members"]
                      if identifier in packages and packages[identifier][0] == row[0]}
        elif row[1] in MANIFEST_KINDS:
            starts = set(context["defaults"])
            if row[1] == "manifest-target-test":
                starts = self._target_owners(context, starts, "test", row[2])
            elif row[1] == "manifest-feature-test":
                starts = {identifier for identifier in context["members"]
                          if identifier in packages and packages[identifier][0] == row[0]}
                starts = self._target_owners(context, starts, "test", row[2].rsplit("@", 1)[0])
            elif row[1] in {"manifest-run", "oracle"}:
                starts = self._target_owners(context, starts, "bin", None)
                if starts is None or len(starts) != 1:
                    return None
        elif row[3] == "features/diverged-ra/check.py":
            # This script actually invokes cargo test --workspace.
            starts = set(context["members"])
        elif row[3] in {"features/inspection/cq-inspection/check.py",
                       "features/inspection/free-variables/check.py"}:
            target = PurePosixPath(row[3]).parent.name
            starts = self._target_owners(context, context["defaults"], "example", target)
        elif row[3] == "features/workloads/check_typed.py":
            starts = self._target_owners(context, context["defaults"], "test", "typed")
        else:
            # The other documented scripts explicitly pass -p.
            starts = {identifier for identifier in context["members"]
                      if identifier in packages and packages[identifier][0] == row[0]}
        if not starts:
            return None
        seen = set()
        pending = list(starts)
        while pending:
            identifier = pending.pop()
            if identifier not in seen:
                seen.add(identifier)
                pending.extend(context["edges"][identifier] - seen)
        roots = {packages[identifier][1] for identifier in seen if identifier in packages}
        if context["historical_lock"] is not None:
            historical = self._historical_scope(context, {packages[identifier][0] for identifier in starts})
            if historical is None:
                return None
            roots.update(historical)
        return roots

    def reasons(self, row):
        row = tuple(row)
        if self.errors:
            return list(self.errors)
        if not self._code_changes:
            return []
        if row not in self._row_contexts:
            return ["row was not part of impact inventory"]
        reasons = []
        if row[3] in self._code_changes:
            reasons.append(f"direct regression source changed: {row[3]}")
        reasons.extend(self._global)
        if row[1] == "scenario" and row[3] in self._non_cargo:
            reasons.append("repository-wide architecture/public API scenario: code/configuration changed")
        contexts = self._row_contexts[row]
        if not contexts and not reasons:
            reasons.append("unknown regression dependency scope")
        for key in contexts:
            context = self._contexts[key]
            if context["error"]:
                reasons.append(f"metadata unavailable for {key[0]} ({key[1]}): {context['error']}")
                continue
            scope = self._scope(row, context)
            if scope is None:
                reasons.append(f"target dependency scope unavailable: {row[0]}")
                continue
            for directory in sorted(scope & self._changed_roots.keys()):
                reasons.append(f"local dependency {directory} changed: {', '.join(self._changed_roots[directory])}")
        return list(dict.fromkeys(reasons))

    def describe(self):
        """Return JSON-serializable selection diagnostics, not a persisted cache."""
        return {
            "since": self.since, "until": self.until, "changed_files": self.changed_files,
            "errors": self.errors,
            "contexts": [{"manifest": manifest, "cwd": cwd, "error": context["error"],
                          "default_packages": sorted(context["packages"][identifier][0]
                                                     for identifier in context["defaults"]
                                                     if identifier in context["packages"]),
                          "local_packages": sorted({directory for _, directory in context["packages"].values()}),
                          "lockfile_local_packages": sorted(context["lock_roots"])}
                         for (manifest, cwd), context in self._contexts.items()],
        }
