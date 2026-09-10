.PHONY: all pdf design check model example clean
all: check pdf
check:
	python3 check.py
pdf:
	latexmk -pdf -interaction=nonstopmode -halt-on-error -outdir=build compiler.tex
	latexmk -pdf -interaction=nonstopmode -halt-on-error -outdir=build physical_design.tex
design:
	latexmk -pdf -interaction=nonstopmode -halt-on-error -outdir=build physical_design.tex
model:
	python3 physical_model.py
example:
	cargo run --manifest-path example/Cargo.toml --locked
	cargo run --manifest-path example/Cargo.toml --locked --bin computed_key
	cargo run --manifest-path example/Cargo.toml --locked --bin argument_placement
	cargo run --manifest-path example/Cargo.toml --locked --bin constraint_planning
	cargo run --manifest-path example/Cargo.toml --locked --bin access_representations
clean:
	latexmk -C -outdir=build compiler.tex
	latexmk -C -outdir=build physical_design.tex
