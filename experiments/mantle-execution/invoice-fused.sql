-- Prepare once per connection: $1 = invoice/worker ID, $2 = unique request sequence.
-- Execute in autocommit with synchronous_commit=on; require one inserted receipt.
WITH totals AS (
    SELECT coalesce(sum(round(coalesce(quantity, 1) * coalesce(amount, 0), 2)), 0) AS total
    FROM bench_invoice_item WHERE invoice_id = $1
), payments AS (
    SELECT coalesce(sum(amount), 0) AS applied FROM bench_payment WHERE invoice_id = $1
), updated AS (
    UPDATE bench_invoice SET total = totals.total, applied = payments.applied,
        unpaid = totals.total - payments.applied
    FROM totals, payments WHERE id = $1
    RETURNING bench_invoice.total, bench_invoice.applied, bench_invoice.unpaid
)
INSERT INTO bench_receipt (worker, request_seq, total, applied, unpaid)
SELECT $1, $2, total, applied, unpaid FROM updated;
