"""Public dependency resolution shared by isolated workload UI consumers."""
import json
import subprocess
from pathlib import Path

PACKAGE = Path(__file__).resolve().parents[3]

def workspace_metadata(cargo, env):
    result = subprocess.run([cargo, "metadata", "--locked", "--offline", "--format-version=1", "--manifest-path", str(PACKAGE / "Cargo.toml")], check=True, capture_output=True, env=env)
    return json.loads(result.stdout)


def dependencies(metadata, names):
    packages = {p["name"]: p for p in metadata["packages"]}
    text = ""
    for name in names:
        package = packages[name]
        spec = f'version = "={package["version"]}"'
        if package["source"] is None:
            spec += ", path = " + json.dumps(str(Path(package["manifest_path"]).parent))
        text += f"{name} = {{ {spec} }}\n"
    return text
