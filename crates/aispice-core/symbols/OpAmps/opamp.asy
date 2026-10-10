Version 4
SymbolType CELL
LINE Normal -24 28 -24 100
LINE Normal -24 28 24 64
LINE Normal -24 100 24 64
LINE Normal 24 64 32 64
LINE Normal -32 48 -24 48
LINE Normal -32 80 -24 80
LINE Normal -19 48 -11 48
LINE Normal -19 80 -11 80
LINE Normal -15 76 -15 84
WINDOW 0 16 32 Left 2
WINDOW 3 16 96 Left 2
SYMATTR Value opamp
SYMATTR Prefix X
SYMATTR SpiceLine Aol=100K
SYMATTR SpiceLine2 GBW=10Meg
SYMATTR Description Ideal single-pole op-amp (aispice ships a matching model for every simulator)
PIN -32 48 NONE 8
PINATTR PinName invin
PINATTR SpiceOrder 1
PIN -32 80 NONE 8
PINATTR PinName noninvin
PINATTR SpiceOrder 2
PIN 32 64 NONE 8
PINATTR PinName out
PINATTR SpiceOrder 3
