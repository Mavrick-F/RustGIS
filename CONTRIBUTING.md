# Contributing to RustGIS

Contributions are welcome. This page covers what to work on and how to propose it. For build,
test, docs and release mechanics see [docs/contributing.md](docs/contributing.md); for code
conventions see [AGENTS.md](AGENTS.md).

## Most roadmap items have no issue

[ROADMAP.md](ROADMAP.md) lists where RustGIS is heading. Only a few of its items link to an
issue; **most features have no issue yet**. Don't wait for one to appear before starting. If
something on the roadmap (or not on it) interests you, propose it and open a pull request.

## How to contribute a feature

1. **Check for an existing issue.** Search the
   [issues](https://github.com/opengeos/RustGIS/issues) and the roadmap. If there is an issue,
   comment on it to say you're taking it.
2. **Write a short proposal** if there is no issue. It covers:
   - the problem or workflow it serves;
   - what you plan to build: the commands it adds, where it sits in the ribbon, which formats or
     tools are involved;
   - which crates it touches and whether it adds a dependency;
   - how you will test it.

   Put it in an issue with the
   [feature request](https://github.com/opengeos/RustGIS/issues/new?template=feature_request.yml)
   form, or in the description of a draft pull request. For changes that need coordination (see
   below) use an issue and wait for a maintainer's reply; otherwise either works.
3. **Open a pull request.** Draft pull requests are welcome early, to get feedback on the
   approach before the work is finished. Link the issue (`Closes #123`) and fill in the
   [pull request template](.github/pull_request_template.md). Before it is ready for review, it
   must meet the rules and quality gates in [AGENTS.md](AGENTS.md).

## Pick freely or coordinate first

- **Pick freely**: roadmap items marked _(open)_ are self-contained. Open a pull request directly,
  with the proposal in its description; an issue is optional.
- **Coordinate first**: open a proposal issue and wait for a maintainer's reply before writing much
  code if your change
  - changes the data model (`Feature`, `Coord`, `Renderer`);
  - adds a dependency or a crate;
  - changes the `.rgis` project format, or an existing command's id or parameters (the CLI,
    Python, MCP and saved scripts call commands by id);
  - changes the plugin interface;
  - falls under _Exploratory_ in the roadmap.

Roadmap themes are not releases or an order; an item moves to Done when it lands. A
pull request that lands a roadmap item, or an item of [GEOLIBRE_PARITY.md](GEOLIBRE_PARITY.md),
updates that file in the same change.
