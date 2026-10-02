-- Return matches both order and item; demographics have two independent roles.
SELECT DISTINCT sale.c0, sale.c1, returned.c5
FROM "mini-linq/raw/0" AS sale
JOIN "mini-linq/raw/1" AS returned ON returned.c0 = sale.c0 AND returned.c1 = sale.c1
JOIN "mini-linq/raw/2" AS page ON page.c0 = sale.c2
JOIN "mini-linq/raw/3" AS refunded_demo ON refunded_demo.c0 = returned.c2
JOIN "mini-linq/raw/3" AS returning_demo ON returning_demo.c0 = returned.c3
JOIN "mini-linq/raw/4" AS address ON address.c0 = returned.c4
JOIN "mini-linq/raw/5" AS sold_date ON sold_date.c0 = sale.c3
JOIN "mini-linq/raw/6" AS reason ON reason.c0 = returned.c5
ORDER BY 1, 2, 3;
