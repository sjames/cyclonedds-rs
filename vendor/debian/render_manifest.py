import argparse
import re
from pathlib import Path


def escape_toml_basic_string(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


PLACEHOLDER_PATTERN = re.compile(r"__[A-Z0-9_]+__")


def render_template(text: str, values: dict[str, str]) -> str:
    rendered = text
    for key, value in values.items():
        rendered = rendered.replace(f"__{key}__", escape_toml_basic_string(value))

    unresolved = sorted(set(PLACEHOLDER_PATTERN.findall(rendered)))
    if unresolved:
        unresolved_text = ", ".join(unresolved)
        raise ValueError(f"Unresolved placeholders found: {unresolved_text}")

    return rendered


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Render Cargo.toml from Cargo-template.toml for cargo-deb"
    )
    parser.add_argument("--template", required=True, help="Source Cargo-template.toml path")
    parser.add_argument("--output", required=True, help="Output Cargo.toml path")
    parser.add_argument("--crate-name", required=True, help="[package].name value")
    parser.add_argument("--crate-version", required=True, help="[package].version value")
    parser.add_argument("--package-name", required=True, help="[package.metadata.deb].name value")
    parser.add_argument("--package-dev-name", required=True, help="[package.metadata.deb.variants.dev].name value")
    parser.add_argument("--package-config-name", required=True, help="[package.metadata.deb.variants.config].name value")
    parser.add_argument("--homepage", required=True, help="[package].homepage value")
    parser.add_argument("--maintainer", required=True, help="[package.metadata.deb].maintainer value")
    parser.add_argument("--copyright", required=True, help="[package.metadata.deb].copyright value")
    args = parser.parse_args()

    source_path = Path(args.template)
    output_path = Path(args.output)
    content = source_path.read_text(encoding="utf-8")

    values = {
        "CRATE_NAME": args.crate_name,
        "CRATE_VERSION": args.crate_version,
        "PACKAGE_NAME": args.package_name,
        "PACKAGE_DEV_NAME": args.package_dev_name,
        "PACKAGE_CONFIG_NAME": args.package_config_name,
        "HOMEPAGE": args.homepage,
        "MAINTAINER": args.maintainer,
        "COPYRIGHT": args.copyright,
    }

    output_path.write_text(render_template(content, values), encoding="utf-8")


if __name__ == "__main__":
    main()
