"""Main entry point for the e-commerce app."""

from models import User, Order
from services import process_order
from pkg.payments import PaymentService


def main():
    user = User(id=1, name="Alice", email="alice@example.com")
    order = Order(user)
    order.add_item(101, 2)

    if PaymentService.validate_card("1234567890123456"):
        tracking = process_order(order)
        print(f"Order submitted: {tracking}")
