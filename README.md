# Market Connector

A Rust-based WebSocket market data connector for cryptocurrency exchanges.

## Disclaimer

**THIS SOFTWARE IS PROVIDED FOR EDUCATIONAL AND DEMONSTRATION PURPOSES ONLY.**

This project is a learning exercise and proof-of-concept implementation. It is **NOT** intended for production use, live trading, or any real financial operations.

By using this software, you acknowledge that:

- This is **not** financial software and should **not** be used for actual trading
- The authors assume **no responsibility** for any financial losses incurred
- This code has **not** been audited for security or correctness
- There are **no guarantees** of reliability, accuracy, or fitness for any purpose
- You use this software **entirely at your own risk**

**Do not use this software with real money or real trading accounts.**

## Overview

This project demonstrates how to build a market data connector in Rust, featuring:

- WebSocket connectivity to Binance public trade streams
- Normalized data model with precise decimal arithmetic
- Automatic reconnection with exponential backoff
- Graceful shutdown handling
- Basic metrics and health reporting

## Project Structure

```
crates/
├── model/             # Normalized data types (Trade, MarketEvent)
├── connector-core/    # Connector traits and configuration
├── connector-binance/ # Binance WebSocket client implementation
├── common/            # Shared utilities (logging, backoff)
├── metrics/           # Metrics collection and health status
└── runner/            # Binary entry point
```

## Building

```bash
cargo build --release
```

## Running

```bash
# Default: BTCUSDT
cargo run --release --bin market-connector

# Multiple symbols
cargo run --release --bin market-connector BTCUSDT ETHUSDT SOLUSDT
```

Press `Ctrl+C` for graceful shutdown with metrics summary.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
