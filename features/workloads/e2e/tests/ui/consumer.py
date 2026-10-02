"""Public dependency resolution shared by isolated workload UI consumers."""
import json
import subprocess
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parents[3]

def dependencies(cargo, names, env):
    result = subprocess.run([cargo, "metadata", "--locked", "--offline", "--format-version=1", "--manifest-path", str(WORKSPACE / "Cargo.toml")], check=True, capture_output=True, env=env)
    packages = {p["name"]: p for p in json.loads(result.stdout)["packages"]}
    text = ""
    for name in names:
        package = packages[name]
        spec = f'version = "={package["version"]}"'
        if package["source"] is None:
            spec += ", path = " + json.dumps(str(Path(package["manifest_path"]).parent))
        text += f"{name} = {{ {spec} }}\n"
    return text
