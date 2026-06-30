"""Payment service."""

from models import User


class PaymentService:
    """Handles payment processing."""

    @staticmethod
    def validate_card(card_number: str) -> bool:
        return len(card_number) == 16

    @classmethod
    def default_provider(cls) -> str:
        return "stripe"

    async def charge(self, user: User, amount: float) -> bool:
        result = await self._do_charge(user, amount)
        return result

    async def _do_charge(self, user: User, amount: float):
        return True
