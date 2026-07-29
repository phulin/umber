% Deterministic system adapter for TeX82 section 1337.
%
% Web2C's onlyTeX build deliberately compiles SOURCE_DATE_EPOCH support out
% of get_date_and_time.  The oracle still needs the same pinned job-start
% clock as Umber's hermetic World, so replace only this explicitly
% system-dependent procedure with the UTC projection of epoch 1783604160:
% 2026-07-09 13:36.

@x [1337] Replace Web2C's host clock with the pinned oracle clock.
@p procedure fix_date_and_time;
begin date_and_time(sys_time,sys_day,sys_month,sys_year);
@y
@p procedure fix_date_and_time;
begin sys_time:=13*60+36;
sys_day:=9; sys_month:=7; sys_year:=2026;
@z
