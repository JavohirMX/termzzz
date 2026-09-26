![Crates.io Total Downloads](https://img.shields.io/crates/d/termzzz)
![Crates.io Downloads (recent)](https://img.shields.io/crates/dr/termzzz)
![GitHub License](https://img.shields.io/github/license/oiwn/termzzz)
[![codecov](https://codecov.io/gh/oiwn/termzzz/graph/badge.svg?token=C7G4AX1ASV)](https://codecov.io/gh/oiwn/termzzz)

# 🦀 TARTS: Terminal Arts 🎨

> **BLAZINGLY FAST** terminal screensavers written in Rust!

`termzzz` (shortcut from **T**erminal **Arts**) is a collection of **MEMORY SAFE**
terminal-based screen savers that bring visual delight to your command line.
Built with **ZERO-COST ABSTRACTIONS**, these screen savers run efficiently while
providing stunning visual effects.

![matrix demo](assets/matrix.gif)

## ✨ Features

- 🌧️ **Matrix Rain**: Experience the famous "Matrix" digital rain effect right in your terminal
- 🧫 **Conway's Game of Life**: Watch the classic cellular automaton evolve before your eyes
- 🧩 **Maze Generation**: Get lost in procedurally generated mazes
- 🐦 **Boids**: Witness the emergent flocking behavior of these simulated birds
- 🧊 **3D Cube**: Renders a rotating 3D cube using terminal graphics with braille patterns for higher resolution
- 🦀 **Crab**: Animated crabs walking across your screen, interacting with each other and the environment
- 🍩 **Rotating Donut**: A mesmerizing rotating donut rendered in the terminal
- 🚰 **Pipes**: Watch pipes flow with a smooth animation
- 🔥 **Fire**: A cozy fireplace effect to warm up your terminal
- ⚡ **Plasma**: Electric plasma effect with vibrant colors and smooth animations
- ✨ **Constellation**: Drifting stars that connect with dotted lines and twinkle
- 🎯 **Blank**: Simple blank screen with minimal resource usage

## 🚀 Installation

### Homebrew (macOS & Linux)
```bash
brew tap oiwn/tap && brew install termzzz
```

### Cargo (Cross-platform)
```bash
cargo install termzzz
```

### Nix

Direct from GitHub (always latest version):
```bash
nix run github:oiwn/termzzz -- matrix
```

Or from the nixpkgs (may be older version):
```bash
nix-shell -p termzzz --run "termzzz matrix"
```

### Manual Download
Download the latest binary from [GitHub Releases](https://github.com/oiwn/termzzz/releases)

## 🛠️ Usage

Run any effect by name:

```bash
termzzz matrix   # The classic digital rain effect
termzzz life     # Conway's Game of Life
termzzz maze     # Watch a maze generate itself
termzzz boids    # Bird-like flocking simulation
termzzz cube     # 3D rotating cube using braille patterns
termzzz crab     # Animated crabs with collisions
termzzz donut    # Rotating donut
termzzz pipes    # Pipes effect
termzzz fire     # Fire effect
termzzz plasma   # Electric plasma effect
termzzz constellation  # Drifting stars and dotted constellations
termzzz blank    # Simple blank screen
```

**Controls:** Press `q`, `Esc`, or `Ctrl+C` to exit

**Quick Test:** Try the most popular effect first!
```bash
termzzz matrix
```

## 🧪 Development

This project uses standard Rust tooling:

```bash
# Build the project
cargo build --release

# Run tests
cargo test

# Benchmark performance
cargo bench
```

## 🤝 Contributing

Contributions are welcome! Please feel free to submit pull requests, report bugs, and suggest features.

## 📜 License

This project is licensed under the [MIT License](https://opensource.org/licenses/MIT).

---

<div align="center">
  <sub>Built with ❤️ and <strong>FEARLESS CONCURRENCY</strong></sub>
</div>
