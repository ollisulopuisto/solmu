# Balanced wrapping

Message text wraps by balancing line lengths instead of filling each line to
the edge, so a bubble avoids a ragged last line or a lone word on its own row.
Consecutive messages from the same sender share one bubble width when wrapping
at that width costs at most one extra line, so a run of short replies lines up
cleanly.

Long Finnish words hyphenate at layout time when the libvoikko library is
available: words longer than ten letters can break at compound joints and
other dictionary points, always leaving at least three letters on each side.
Without libvoikko the same text simply wraps without hyphens. Links are never
hyphenated: one that fits the bubble's maximum width stays whole on its own
line, and a longer one breaks at punctuation only.

Inserted hyphens and line breaks exist only on screen. Copying a message, in
any selection, always yields the original text.
