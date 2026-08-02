# umber2-johp.456 — constructed-leader glue trace boundary

Authority: TeX82 §§1030 and 1078.

After a constructed leader box closes, `box_end` fetches the following glue
operand inside the leader case. That fetch does not return to §1030's
`big_switch` label and therefore receives no settled unexpandable-command
trace. Expansion tracing performed while fetching the operand remains live.

Canonical replay splits box construction and completion across processor
episodes. Its pending-leader state preserves the §1078 internal-fetch boundary
across that split; only commands without such a pending leader reach §1030's
main-control trace boundary.
