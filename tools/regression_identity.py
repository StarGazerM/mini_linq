"""Fail-closed identities for log-backed regression reuse, not execution evidence.

Identity(root).fingerprint(row) returns {"version": 1, "key": sha256} or None;
last_reason explains a refusal. Cargo's actual unstable unit graph establishes
which targets/features/profiles run. Root targets and local runtime libraries
that can invoke local procedural macros are expanded in matching unit contexts;
all linked runtime sources and Cargo build-script outputs remain key inputs.
Doctests, native linkage, custom runners and dynamic compiler targets fail closed.
Sources/environment are rehashed on every call. Procedural macros, build scripts
and included inputs require fresh expansions. Documentation-only inputs are
excluded, while included documentation is observed in the expanded Rust.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

import expanded_gate as gate

RUNTIME_OBSERVER = re.compile(
    r"\b(?:trybuild|compiletest|Command|dlopen|libloading|rustc|fs|net)\s*:\s*:"
    r"|\b(?:std|alloc)\s*:\s*:\s*(?:process|fs|net)\b"
    r"|\b(?:use|extern\s+crate)\s+(?:trybuild|compiletest|libloading)\b"
    r"|\buse\b[^;]*\b(?:trybuild|compiletest|Command|libloading|process|fs|net)\b"
    r"|\b(?:dlopen|system|popen|execve|execl|fork)\s*\("
    r"|\b\w*compile_fail\w*\s*(?:!|\()"
    r'|extern\s+"[^"]+"\s*\{|#\s*\[\s*link\b'
)


def runtime_observer(text):
    """Inspect lexical paths/calls, not identifier names or opaque prose.

    Keep punctuation (and string literals for extern ABIs), discard comments.
    Thus a harness description, local `fs` variable, or `rustc_test_marker`
    attribute is not mistaken for a filesystem/compiler invocation.
    """
    lexemes = [value for kind, _, _, value in gate.tokens(text)
               if not (kind == "opaque" and value.startswith(("//", "/*")))]
    # Opaque literals cannot contribute apparent code, except extern ABI strings.
    code = []
    previous = None
    for value in lexemes:
        literal = value.startswith(('"', 'r"', 'r#', 'b"', 'br"', 'br#'))
        code.append(value if not literal or previous == "extern" else '""')
        previous = value
    match = RUNTIME_OBSERVER.search(" ".join(code))
    return match.group(0) if match else None


class Unproven(RuntimeError):
    pass


def _command(row):
    package, kind, target, source = row
    if kind in {"test", "example"}:
        return ["cargo", "test", "--locked", "-p", package, "--all-features", f"--{kind}", target]
    if kind == "lib":
        return ["cargo", "test", "--locked", "-p", package, "--lib"]
    if kind not in {"manifest-target-test", "manifest-test", "manifest-run", "oracle", "oracle-test"}:
        raise Unproven(f"unsupported regression kind: {kind}")
    command = ["cargo", "run" if kind in {"manifest-run", "oracle"} else "test",
               "--locked", "--manifest-path", source]
    if kind == "manifest-target-test":
        command += ["--test", target]
    if kind in {"oracle", "oracle-test"}:
        command += ["--features", "oracle"]
    if kind == "oracle":
        command += ["--", target]
    return command


def _compile_command(command):
    return command[:command.index("--")] if "--" in command else list(command)


class Identity:
    def __init__(self, root: Path):
        self.root = Path(root).resolve()
        self.last_reason = None
        self._cache = {}
        self._requires_expansion = False
        self._target_directory = None

    def _portable(self, value):
        if isinstance(value, str):
            return value.replace(str(self.root), "<TREE>")
        if isinstance(value, dict):
            return {self._portable(k): self._portable(v) for k, v in value.items()}
        if isinstance(value, (list, tuple)):
            return [self._portable(item) for item in value]
        return value

    def _run(self, command, env):
        result = subprocess.run(command, cwd=self.root, env=env, capture_output=True, text=True)
        if result.returncode:
            raise Unproven(f"{' '.join(command)}: {result.stderr.strip()[-2000:]}")
        return result.stdout

    def _graph(self, command, env):
        graph_command = _compile_command(command)
        if graph_command[1] == "run":
            # `cargo run --unit-graph` panics on some Cargo versions. Build has
            # the same dev-mode units; extra default bins/libs only overselect.
            graph_command[1] = "build"
        graph = json.loads(self._run(graph_command +
                          ["--offline", "--unit-graph", "-Z", "unstable-options"], env))
        if graph.get("version") != 1 or not graph.get("roots") or not graph.get("units"):
            raise Unproven("unsupported or empty Cargo unit graph")
        return graph

    def _closure(self, graph, index):
        """Full unit signature, including extern names and host/target dependencies."""
        units = graph["units"]
        memo = {}
        def visit(n):
            if n not in memo:
                unit = units[n]
                fields = {key: unit[key] for key in
                          ("pkg_id", "target", "profile", "platform", "mode", "features")}
                # Cargo calls equivalent dev settings "test" for test dependencies.
                # The name itself is not a rustc flag; every actual setting stays.
                fields["profile"] = {k: v for k, v in unit["profile"].items() if k != "name"}
                fields["dependencies"] = sorted([
                    {**{k: v for k, v in dependency.items() if k != "index"},
                     "unit": visit(dependency["index"])}
                    for dependency in unit["dependencies"]], key=gate.canonical)
                memo[n] = gate.cache_key(self._portable(fields))
            return memo[n]
        return visit(index)

    @staticmethod
    def _runtime_units(graph):
        units = graph["units"]
        pending = list(graph["roots"])
        roots = set(pending)
        seen = set()
        while pending:
            index = pending.pop()
            if index in seen:
                continue
            unit = units[index]
            kind = unit["target"]["kind"]
            if index not in roots and "proc-macro" in kind:
                continue
            if "custom-build" in kind or unit["mode"] == "run-custom-build":
                continue
            if unit["mode"] not in {"build", "test"}:
                raise Unproven(f"unsupported Cargo unit mode: {unit['mode']} (including doctests)")
            if not set(kind) <= {"lib", "rlib", "proc-macro", "bin", "test", "example"}:
                raise Unproven(f"unsupported Rust target kind: {kind}")
            seen.add(index)
            pending.extend(dependency["index"] for dependency in unit["dependencies"])
        return sorted(seen)

    def _configs(self, env):
        paths = {parent / ".cargo" / name for parent in (self.root, *self.root.parents)
                 for name in ("config", "config.toml")}
        home = Path(env.get("CARGO_HOME", str(Path.home() / ".cargo")))
        paths.update(home / name for name in ("config", "config.toml"))
        values = {}
        for path in sorted(paths):
            if not path.is_file():
                continue
            data = path.read_bytes()
            config = tomllib.loads(data.decode())
            # These can execute arbitrary programs or inject native build state.
            if any(key in config for key in ("env", "source", "paths", "unstable")):
                raise Unproven(f"unsupported Cargo configuration: {path}")
            if any(key in config.get("build", {}) for key in
                   ("rustc", "rustc-wrapper", "rustc-workspace-wrapper", "rustdoc")):
                raise Unproven(f"custom compiler/wrapper configuration: {path}")
            for target in config.get("target", {}).values():
                if any(key not in {"rustflags", "rustdocflags"} for key in target):
                    raise Unproven(f"runner/linker/native target configuration: {path}")
            values[str(path)] = gate.sha(data)
        return values

    def _environment(self):
        env = os.environ.copy()
        for key, value in env.items():
            if value and (key in {"RUSTC", "RUSTDOC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"}
                          or re.fullmatch(r"CARGO_TARGET_.*_(RUNNER|LINKER)", key)):
                raise Unproven(f"custom tool or runner environment: {key}")
        return env

    def _files(self, packages, runtime_ids):
        """Hash actual bytes, not mtimes; nested packages have independent ownership."""
        directories = sorted([(Path(p["manifest_path"]).parent.resolve(), p["id"])
                              for p in packages.values()], key=lambda item: len(item[0].parts), reverse=True)
        compiler_ids = {key for key, package in packages.items()
                        if key not in runtime_ids and package.get("source") is None}
        files, comparable = {}, {}
        roots = [self.root]
        for directory, _ in directories:
            if not directory.is_relative_to(self.root) and not any(directory.is_relative_to(p) for p in roots):
                roots.append(directory)
        for base in roots:
            for directory, dirs, names in os.walk(base):
                dirs[:] = sorted(d for d in dirs if d not in gate.EXCLUDED)
                for name in dirs:
                    if (Path(directory) / name).is_symlink():
                        raise Unproven(f"symlinked input directory: {Path(directory) / name}")
                for name in sorted(names):
                    path = Path(directory) / name
                    if path.is_symlink() or not path.is_file():
                        raise Unproven(f"non-regular input: {path}")
                    # Git worktree administrative files are not test inputs.
                    if path.name == ".git" or gate.documentation_path(path.relative_to(base)):
                        continue
                    data = path.read_bytes()
                    if path.suffix == ".rs" and re.search(
                            rb"\b(include|include_str|include_bytes)\s*!|#\s*\[\s*path\s*=", data):
                        # Included files can be outside the repository inventory.
                        # Re-expansion observes their contents on every call.
                        self._requires_expansion = True
                    label = str(path)
                    files[label] = gate.sha(data)
                    owner = next((identifier for parent, identifier in directories
                                  if path.is_relative_to(parent)), None)
                    if not (owner in compiler_ids and path.suffix == ".rs"):
                        comparable[label] = files[label]
        return files, comparable

    def _metadata(self, row, env):
        manifest = "Cargo.toml" if row[1] in {"test", "example", "lib"} else row[3]
        command = ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
                   "--manifest-path", manifest]
        if row[1] in {"test", "example"}:
            command += ["--all-features"]
        elif row[1] in {"oracle", "oracle-test"}:
            command += ["--features", "oracle"]
        metadata = json.loads(self._run(command, env))
        target_directory = metadata.get("target_directory")
        if target_directory:
            self._target_directory = str(Path(target_directory) / "regression-identity")
        if row[1] in {"manifest-test", "oracle-test"}:
            selected = metadata.get("workspace_default_members")
            if selected is None:
                selected = [metadata.get("resolve", {}).get("root")]
            if not selected or None in selected:
                raise Unproven("unknown default workspace test selection")
            for package in metadata["packages"]:
                if package["id"] in selected and any(target.get("doctest", False)
                                                     for target in package["targets"]):
                    raise Unproven("manifest-wide Cargo tests include unsupported doctests")
        return {p["id"]: p for p in metadata["packages"]}

    def _expansion_command(self, unit, package):
        target = unit["target"]
        kinds = target["kind"]
        if "test" in kinds:
            selector = ["--test", target["name"]]
        elif "example" in kinds:
            selector = ["--example", target["name"]]
        elif "bin" in kinds:
            selector = ["--bin", target["name"]]
        else:
            selector = ["--lib"]
        command = ["cargo", "rustc", "--locked", "--manifest-path", package["manifest_path"],
                   "-p", package["name"], "--no-default-features", *selector,
                   "--profile", "test" if unit["mode"] == "test" else "dev"]
        if unit["features"]:
            command += ["--features", ",".join(sorted(unit["features"]))]
        if unit["platform"]:
            command += ["--target", unit["platform"]]
        return command

    def _build_state(self, records, packages):
        state = []
        for record in records:
            identifier = record["package_id"]
            if identifier not in packages:
                raise Unproven("unknown package in build-script output")
            if record.get("linked_libs"):
                raise Unproven("native linked library identity is unproven")
            directory = Path(record["out_dir"])
            if not directory.is_dir():
                raise Unproven("missing Cargo build-script output directory")
            if any(not Path(path).is_relative_to(directory) for path in record.get("linked_paths", [])):
                raise Unproven("external native build-script search path")
            output = directory.parent / "output"
            if not output.is_file():
                raise Unproven("missing Cargo build-script declarations")
            declarations = output.read_text()
            generated = {}
            for path in sorted(directory.rglob("*")):
                if path.is_symlink():
                    raise Unproven("symlinked generated build input")
                if path.is_file():
                    generated[path.relative_to(directory).as_posix()] = gate.sha(path.read_bytes())
            inputs = {}
            for line in declarations.splitlines():
                match = re.match(r"cargo::?rerun-if-changed=(.*)", line)
                if not match:
                    continue
                path = Path(packages[identifier]["manifest_path"]).parent / match.group(1)
                if path.is_symlink() or not path.exists():
                    raise Unproven(f"missing or symlinked declared build input: {path}")
                members = sorted(path.rglob("*")) if path.is_dir() else [path]
                for member in members:
                    if member.is_symlink():
                        raise Unproven(f"symlinked declared build input: {member}")
                    if member.is_file():
                        inputs[str(member)] = gate.sha(member.read_bytes())
            state.append(self._portable({
                "package": identifier, "cfgs": sorted(record.get("cfgs", [])),
                "env": sorted(record.get("env", [])), "generated": generated,
                "declarations": declarations.replace(str(directory), "<OUT>"),
                "inputs": inputs,
            }))
        return sorted(state, key=gate.canonical)

    def _expanded_output(self, command, env):
        text = self._run(command + ["--offline", "--message-format=json", "--", "-Zunpretty=expanded"], env)
        rust, scripts = [], []
        for line in text.splitlines(keepends=True):
            try:
                message = json.loads(line)
            except ValueError:
                rust.append(line)
                continue
            if not isinstance(message, dict) or "reason" not in message:
                rust.append(line)
            elif message["reason"] == "build-script-executed":
                scripts.append(message)
        return "".join(rust), scripts

    @staticmethod
    def _needs_expansion(graph, index, packages):
        if index in graph["roots"]:
            return True
        pending, seen = [(index, False)], set()
        while pending:
            current, compiler = pending.pop()
            if (current, compiler) in seen:
                continue
            seen.add((current, compiler))
            dependency = graph["units"][current]
            compiler = compiler or "proc-macro" in dependency["target"]["kind"]
            if compiler and packages[dependency["pkg_id"]].get("source") is None:
                return True
            pending.extend((item["index"], compiler) for item in dependency["dependencies"])
        return False

    def fingerprint(self, row):
        self.last_reason = None
        try:
            return self._fingerprint(tuple(row))
        except Exception as error:
            # Identity is optional optimization; ordinary execution owns errors.
            self.last_reason = f"{type(error).__name__}: {error}"
            return None

    def _fingerprint(self, row):
        command = _command(row)
        if not (self.root / row[3]).is_file():
            raise Unproven(f"missing regression source: {row[3]}")
        original_env = self._environment()
        env = {**original_env, "RUSTC_BOOTSTRAP": "1"}
        configs = self._configs(original_env)
        toolchain = {tool: self._run([tool, "-vV" if tool == "rustc" else "--version"], env)
                     for tool in ("rustc", "cargo", "rustfmt")}
        packages = self._metadata(row, env)
        if self._target_directory:
            env["CARGO_TARGET_DIR"] = self._target_directory
        graph = self._graph(command, env)
        runtime = self._runtime_units(graph)
        for index in graph["roots"]:
            source = Path(graph["units"][index]["target"]["src_path"]).read_text()
            observer = runtime_observer(source)
            if observer:
                raise Unproven(f"selected target observes dynamic compilation or external runtime inputs: {observer}")
        runtime_ids = {graph["units"][n]["pkg_id"] for n in runtime}
        all_ids = {unit["pkg_id"] for unit in graph["units"]}
        if not all_ids <= packages.keys():
            raise Unproven("Cargo metadata/unit package identities disagree")
        # Only graph-reachable packages may be treated as compiler-only. Other
        # workspace packages and fixtures remain conservative runtime inputs.
        packages = {key: packages[key] for key in all_ids}
        self._requires_expansion = False
        files, inputs = self._files(packages, runtime_ids)
        context = {"row": row, "command": command, "toolchain": toolchain,
                   "environment": gate.cache_key(self._portable(original_env)), "config": configs,
                   "units": graph,
                   "dependencies": sorted([[key, p["version"], p.get("source")]
                                            for key, p in packages.items()]),
                   "normalizer": gate.TOOL_DIGEST,
                   "identity": gate.sha(Path(__file__).read_bytes())}
        cache_key = gate.cache_key({"context": context, "files": files})
        # Procedural macros and compiler build scripts can read external state.
        # Their freshly emitted Rust, not a source-only cache hit, is the proof.
        cacheable = not self._requires_expansion and not any(
            set(unit["target"]["kind"]) & {"custom-build", "proc-macro"}
            for unit in graph["units"])
        if cacheable and cache_key in self._cache:
            return dict(self._cache[cache_key])
        expanded = []
        build_records = []
        for index in sorted(runtime, key=lambda item: (item not in graph["roots"], item)):
            if not self._needs_expansion(graph, index, packages):
                continue
            unit = graph["units"][index]
            package = packages[unit["pkg_id"]]
            expansion_command = self._expansion_command(unit, package)
            if index in graph["roots"] and len(graph["roots"]) == 1:
                # Keep the original workspace/default-feature selection for the
                # selected target, instead of resolving its standalone manifest.
                expansion_command = ["cargo", "rustc", *_compile_command(command)[2:]]
                if "-p" not in expansion_command:
                    expansion_command += ["-p", package["name"]]
                if not any(flag in expansion_command for flag in ("--lib", "--bin", "--test", "--example")):
                    expansion_command += (["--lib"] if "lib" in unit["target"]["kind"]
                                          else ["--bin", unit["target"]["name"]])
                expansion_command += ["--profile", "test" if unit["mode"] == "test" else "dev"]
            signature = self._closure(graph, index)
            alternatives = [expansion_command]
            if index not in graph["roots"]:
                manifest = (_compile_command(command)[_compile_command(command).index("--manifest-path") + 1]
                            if "--manifest-path" in command else "Cargo.toml")
                # Selecting a dependency in the original workspace preserves
                # feature unification/profile overrides. Still prove the graph.
                alternatives.insert(0, [
                    "cargo", "rustc", "--locked", "--manifest-path", manifest,
                    "-p", package["name"], "--lib", "--profile", "dev"])
            for alternative in alternatives:
                try:
                    candidate = self._graph(alternative, env)
                except Unproven:
                    continue
                if (len(candidate["roots"]) == 1
                        and self._closure(candidate, candidate["roots"][0]) == signature):
                    expansion_command = alternative
                    break
            else:
                raise Unproven(f"cannot reproduce exact feature/profile/dependency unit: {unit['pkg_id']} / {unit['target']['name']}")
            # --profile test selects Cargo's test mode and --test harness flag;
            # the graph comparison above rejects a non-harness reproduction.
            text, records = self._expanded_output(expansion_command, env)
            build_records.extend(records)
            if not text.strip():
                raise Unproven("empty expanded Rust")
            normalized = gate.normalize(text, self.root, self.root / "target/regression-identity", "rustfmt")
            observer = runtime_observer(normalized)
            if index in graph["roots"] and observer:
                raise Unproven(f"expanded Rust has unbounded runtime inputs: {observer}")
            expanded.append([signature, gate.sha(normalized.encode())])
        expected_scripts = {
            graph["units"][dependency["index"]]["pkg_id"]
            for index in runtime for dependency in graph["units"][index]["dependencies"]
            if graph["units"][dependency["index"]]["mode"] == "run-custom-build"}
        if not expected_scripts <= {record["package_id"] for record in build_records}:
            raise Unproven("missing linked runtime build-script execution records")
        build_state = self._build_state(build_records, packages)
        after_files, _ = self._files(packages, runtime_ids)
        if (after_files != files or os.environ.copy() != original_env
                or self._configs(original_env) != configs
                or self._build_state(build_records, packages) != build_state):
            raise Unproven("inputs changed while obtaining expansion identity")
        value = {"version": 1, "key": gate.cache_key(self._portable({
            "context": context, "inputs": inputs, "expanded": expanded, "build_scripts": build_state}))}
        if cacheable:
            self._cache[cache_key] = value
        return dict(value)
