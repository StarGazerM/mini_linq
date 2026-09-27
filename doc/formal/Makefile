.PHONY: all pdf check example clean
all: check pdf
check:
	python3 check.py
pdf:
	latexmk -pdf -interaction=nonstopmode -halt-on-error -outdir=build compiler.tex
example:
	cargo run --manifest-path example/Cargo.toml --locked
	cargo run --manifest-path example/Cargo.toml --locked --bin computed_key
	cargo run --manifest-path example/Cargo.toml --locked --bin argument_placement
	cargo run --manifest-path example/Cargo.toml --locked --bin constraint_planning
clean:
	latexmk -C -outdir=build compiler.tex
