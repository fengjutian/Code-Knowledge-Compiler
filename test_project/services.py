"""Order processing utilities."""

from models import Order


def process_order(order: Order) -> str:
    """Submit an order for processing and return a tracking ID."""
    return f"ORD-{order.total_items()}"


def cancel_order(order: Order):
    """Cancel a pending order."""
    pass
