SELECT DISTINCT t.tag, p.productkey, p.price, p.released
FROM Selected AS s JOIN Tag AS t USING (tag)
JOIN Product AS p USING (productkey);
