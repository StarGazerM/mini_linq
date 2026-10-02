#!/usr/bin/env python3
"""Compare reviewed core API snapshots with rustdoc JSON (pinned toolchain).

Run --update only while reviewing an intentional API change. CI/default mode
never overwrites snapshots. IDs, spans, documentation and bodies are excluded;
exported declarations, fields, generics, traits, implementations and reexports
are retained. The JSON payloads are signatures, not a source-code grep.
"""
from __future__ import annotations
import argparse
import difflib
import json
import os
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parents[1]
SNAPSHOTS=ROOT/"tools/public-api"


def snapshot(data):
    index=data["index"]; paths=data["paths"]; root=index[str(data["root"])]; crate=root["name"]
    names={}; exported=set(); rows={}; impls=set()
    def walk(identifier, path, inherited=False):
        identifier=str(identifier)
        if identifier not in index:return
        item=index[identifier]
        if item["crate_id"]!=0:return
        if not inherited and item["visibility"]!="public":return
        names[identifier]=path;exported.add(identifier)
        kind,content=next(iter(item["inner"].items()))
        if kind=="module":
            for child in content["items"]:
                child_item=index.get(str(child)); name=child_item and child_item["name"]
                if name:walk(child,path+"::"+name)
        elif kind=="struct":
            fields=content["kind"]
            children=fields.get("plain",{}).get("fields",[]) if isinstance(fields,dict) else []
            if isinstance(fields,dict) and "tuple" in fields:children=fields["tuple"]
            for child in children:
                if child is not None:walk(child,path+"::"+(index[str(child)]["name"] or str(children.index(child))))
            impls.update(map(str,content["impls"]))
        elif kind=="enum":
            for child in content["variants"]:walk(child,path+"::"+index[str(child)]["name"],True)
            impls.update(map(str,content["impls"]))
        elif kind=="variant":
            shape=content["kind"];children=[]
            if isinstance(shape,dict):children=shape.get("tuple",shape.get("struct",{}).get("fields",[]))
            for child in children:
                if child is not None:walk(child,path+"::"+(index[str(child)]["name"] or str(children.index(child))),True)
        elif kind=="trait":
            for child in content["items"]:walk(child,path+"::"+index[str(child)]["name"],True)
            impls.update(map(str,content["implementations"]))
    walk(data["root"],crate,True)
    def reference(identifier):
        key=str(identifier)
        if key in names:return names[key]
        if key in paths:return "::".join(paths[key]["path"])
        item=index.get(key)
        if item and item.get("name"):return crate+"::"+item["name"]
        return "<stripped>"
    def normalize(value,key=None):
        if isinstance(value,dict):
            return {k:normalize(v,k) for k,v in sorted(value.items())
                if k not in {"span","docs","links","impls","implementations","has_body","body"}}
        if isinstance(value,list):
            if key in {"items","fields","variants"}:return [reference(v) if v is not None else None for v in value]
            return [normalize(v) for v in value]
        if key=="id":return reference(value)
        if key=="value" and isinstance(value,str):return value
        return value
    # Trait implementations are public when their trait or their self type is
    # exported. A private implementation helper does not become a public API.
    for identifier in sorted(impls):
        item=index.get(identifier)
        if not item or "impl" not in item["inner"]:continue
        content=item["inner"]["impl"]
        if content.get("blanket_impl") is not None:continue
        typ=content["for"];trait=content.get("trait")
        local=typ.get("resolved_path",{}).get("id")
        if local is not None and index.get(str(local),{}).get("crate_id")==0 and str(local) not in exported:continue
        signature=normalize({k:v for k,v in content.items() if k!="items"})
        label=crate+"::<impl "+json.dumps(normalize(typ),sort_keys=True,separators=(",",":"))+" as "+json.dumps(normalize(trait),sort_keys=True,separators=(",",":"))+">"
        rows[label]=signature
        for child in content["items"]:
            child_item=index[str(child)]
            if child_item["visibility"]=="public" or trait is not None:
                name=child_item["name"]
                rows[label+"::"+name]=normalize(child_item["inner"])
    for identifier in exported:
        item=index[identifier]
        kind,content=next(iter(item["inner"].items()))
        if kind=="module":continue
        rows[names[identifier]]=normalize(item["inner"])
    return "\n".join(path+"\t"+json.dumps(sig,sort_keys=True,separators=(",",":")) for path,sig in sorted(rows.items()))+"\n"


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--update",action="store_true");args=parser.parse_args()
    data=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version=1","--locked"],cwd=ROOT))
    packages=sorted(p["name"] for p in data["packages"] if Path(p["manifest_path"]).parent.parent.name in {"languages","passes","runtime","facade","testkit"})
    env=os.environ.copy();env["RUSTC_BOOTSTRAP"]="1"
    logdir=ROOT/"target/public-api-logs";logdir.mkdir(parents=True,exist_ok=True)
    # cargo rustdoc supports a single package; reuse artifacts between packages.
    expected=set()
    for package in packages:
        with (logdir/(package+".log")).open("w") as log:
            subprocess.run(["cargo","rustdoc","-p",package,"--lib","--all-features","--locked","--","-Z","unstable-options","--output-format","json"],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        current=snapshot(json.loads((ROOT/"target/doc"/(package.replace("-","_")+".json")).read_text()))
        path=SNAPSHOTS/(package+".txt");expected.add(path.name)
        if args.update:SNAPSHOTS.mkdir(parents=True,exist_ok=True);path.write_text(current)
        elif not path.exists() or path.read_text()!=current:
            previous=path.read_text() if path.exists() else ""
            diff="".join(difflib.unified_diff(previous.splitlines(True),current.splitlines(True),fromfile=str(path),tofile="current rustdoc API"))
            (logdir/(package+".diff")).write_text(diff)
            raise SystemExit(f"Unreviewed public API change: {package}; see {logdir/(package+'.diff')}")
    extras={p.name for p in SNAPSHOTS.glob("*.txt")}-expected
    if extras:
        if args.update:
            for name in extras: (SNAPSHOTS/name).unlink()
        else: raise SystemExit(f"obsolete API snapshots: {sorted(extras)}")
    print(f"PASS: {len(packages)} public API snapshots "+("updated for review" if args.update else "unchanged"))


if __name__=="__main__":main()
