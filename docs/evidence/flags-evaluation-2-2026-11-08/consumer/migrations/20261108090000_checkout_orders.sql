-- Accepted checkouts and the receipt each one owes.
--
-- `express_receipt_pin` is the batter-flags `Pinned<Toggle>` envelope taken from
-- the accepting request's view; the receipt job replays it instead of reading
-- the flag again. A receipt is either pending (both receipt columns NULL) or
-- sent (both set); the CHECK makes any other combination unrepresentable.
CREATE TABLE checkout_orders (
    order_ref text PRIMARY KEY CHECK (order_ref ~ '^[A-Za-z0-9_-]{1,64}$'),
    recipient text NOT NULL CHECK (length(recipient) BETWEEN 3 AND 254),
    express_receipt_pin jsonb NOT NULL CHECK (jsonb_typeof(express_receipt_pin) = 'object'),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    receipt_mode text CHECK (receipt_mode IN ('express', 'standard')),
    receipt_sent_at timestamptz,
    CONSTRAINT checkout_orders_receipt_state
        CHECK ((receipt_mode IS NULL) = (receipt_sent_at IS NULL))
);
