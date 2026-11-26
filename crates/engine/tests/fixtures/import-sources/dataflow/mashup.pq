section Section1;
shared Products = let
    Source = #table(type table [Sku = text, Price = number], {{"A", 1.5}})
in
    Source;
shared Helper = "only a helper";
