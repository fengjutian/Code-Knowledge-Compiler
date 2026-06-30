"""Core models for the e-commerce system."""

from dataclasses import dataclass


@dataclass
class User:
    """Represents a registered user."""
    id: int
    name: str
    email: str

    def is_active(self) -> bool:
        return True


class Order:
    """Manages order lifecycle."""

    STATUS_PENDING = "pending"
    STATUS_SHIPPED = "shipped"

    def __init__(self, user: User):
        self.user = user
        self.items: list = []

    def add_item(self, product_id: int, quantity: int):
        self.items.append({"product_id": product_id, "quantity": quantity})

    def total_items(self) -> int:
        return len(self.items)

    def submit(self):
        return process_order(self)
