import os, random
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
os.makedirs(out, exist_ok=True)
with open(os.path.join(out, "plain.txt"), "w") as f:
    f.writelines(f"{i}\n" for i in range(1, 3_000_001))
random.seed(1)
words = ["error", "warning", "build", "Compiling", "serde", "tokio", "✓", "→", "λ", "日本語", "🚀", "ok"]
with open(os.path.join(out, "color.txt"), "w") as f:
    for i in range(300_000):
        parts = [f"\x1b[38;5;{random.randint(0, 255)}m{random.choice(words)}\x1b[0m" for _ in range(8)]
        if i % 7 == 0:
            parts.append(f"\x1b[48;2;{i % 255};40;90m bg \x1b[0m")
        f.write(" ".join(parts) + "\n")
