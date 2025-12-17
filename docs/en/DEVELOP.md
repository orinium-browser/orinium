# Developer Docs
This document contains resources useful for developers working on Orinium.

## 🧪 Development test utilities (Examples)
`examples/tests.rs` includes development tests that allow you to exercise Orinium Browser's major components individually.
You can use these to inspect integration between GUI, network, HTML parser, and other subsystems.
> [!WARNING]
> Examples and commands listed here may be out of date or removed. Before using them, check available commands with:
> ```bash
> cargo run --example tests help
> ```

### How to run
```bash
cargo run --example tests help
```

### Example commands
| Command           | Description                                           |
|-------------------|-------------------------------------------------------|
| `help`            | Show available commands                               |
| `fetch_url <URL>` | Fetch the given URL and print the response            |
| `parse_dom <URL>` | Fetch HTML from the URL, build and print the DOM tree |

#### Sample usage
```bash
# Network fetch test
cargo run --example tests fetch_url https://example.com

# DOM parse test
cargo run --example tests parse_dom https://example.com
```

This example harness is intended to make it easy to exercise async and GUI code that is harder to run inside `#[test]` unit tests.
