"""Self-contained module for call graph verification."""


def helper(x: int) -> int:
    """Doubles the input."""
    return x * 2


def compute(a: int, b: int) -> int:
    """Compute a value using helper."""
    result = helper(a)
    return result + b


class Calculator:
    """Simple calculator."""

    def add(self, x: int, y: int) -> int:
        return x + y

    def multiply(self, x: int, y: int) -> int:
        return x * y

    def compute_sum(self, values: list) -> int:
        """Sum all values using add."""
        total = 0
        for v in values:
            total = self.add(total, v)
        return total
