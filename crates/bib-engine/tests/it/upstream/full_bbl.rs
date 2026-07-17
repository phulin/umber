// Direct xfail translation of upstream t/full-bbl.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use super::xfail_upstream;

const UPSTREAM_SOURCE: &str = r########"# -*- cperl -*-
use v5.24;
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More;

if ($ENV{BIBER_DEV_TESTS}) {
  plan tests => 5;
}
else {
  plan skip_all => 'BIBER_DEV_TESTS not set';
}

use IPC::Run3;
use File::Temp;
use File::Compare;
use File::Which;


my $perl = which('perl');

my $tmpfile = File::Temp->new();
# my $tmpfile = File::Temp->new(UNLINK => 0);
my $bbl = $tmpfile->filename;
my $stdout;
my $stderr;

run3  [ $perl, 'bin/biber', '--noconf', '--nolog', "--output-file=$bbl", 't/tdata/full-bbl.bcf' ], \undef, \$stdout, \$stderr;
# say $stdout;
# say $stderr;

is($? >> 8, 0, 'Full test has zero exit status');
ok(compare($bbl, 't/tdata/full-bbl.bbl') == 0, 'Testing lossort case and sortinit for macros');
like($stdout, qr|WARN - Duplicate entry key: 'F1' in file 't/tdata/full-bbl\.bib', skipping \.\.\.|ms, 'Testing duplicate/case key warnings - 1');
like($stdout, qr|WARN - Possible typo \(case mismatch\) between datasource keys: 'f1' and 'F1' in file 't/tdata/full-bbl\.bib'|ms, 'Testing duplicate/case key warnings - 2');
like($stdout, qr|WARN - Possible typo \(case mismatch\) between citation and datasource keys: 'C1' and 'c1' in file 't/tdata/full-bbl\.bib'|ms, 'Testing duplicate/case key warnings - 3');
"########;
// The upstream subprocess assertions below are expressed as expectations on
// one in-process bibliography session: status, output bytes, and diagnostics.

#[test]
fn assertion_001_full_test_has_zero_exit_status() {
    xfail_upstream(
        "Full test has zero exit status",
        r########"in_process_session.exit_status /* upstream: $? >> 8 */"########,
        r########"0"########,
        r########"is($? >> 8, 0, 'Full test has zero exit status');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_testing_lossort_case_and_sortinit_for_macros() {
    xfail_upstream(
        "Testing lossort case and sortinit for macros",
        r########"in_process_session.output_bytes /* upstream: compare($bbl, 't/tdata/full-bbl.bbl') == 0 */"########,
        r########"fixture_bytes('t/tdata/full-bbl.bbl')"########,
        r########"ok(compare($bbl, 't/tdata/full-bbl.bbl') == 0, 'Testing lossort case and sortinit for macros');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_testing_duplicate_case_key_warnings_1() {
    xfail_upstream(
        "Testing duplicate/case key warnings - 1",
        r########"in_process_session.rendered_diagnostics /* upstream: $stdout */"########,
        r########"qr|WARN - Duplicate entry key: 'F1' in file 't/tdata/full-bbl\.bib', skipping \.\.\.|ms"########,
        r########"like($stdout, qr|WARN - Duplicate entry key: 'F1' in file 't/tdata/full-bbl\.bib', skipping \.\.\.|ms, 'Testing duplicate/case key warnings - 1');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_testing_duplicate_case_key_warnings_2() {
    xfail_upstream(
        "Testing duplicate/case key warnings - 2",
        r########"in_process_session.rendered_diagnostics /* upstream: $stdout */"########,
        r########"qr|WARN - Possible typo \(case mismatch\) between datasource keys: 'f1' and 'F1' in file 't/tdata/full-bbl\.bib'|ms"########,
        r########"like($stdout, qr|WARN - Possible typo \(case mismatch\) between datasource keys: 'f1' and 'F1' in file 't/tdata/full-bbl\.bib'|ms, 'Testing duplicate/case key warnings - 2');"########,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_testing_duplicate_case_key_warnings_3() {
    xfail_upstream(
        "Testing duplicate/case key warnings - 3",
        r########"in_process_session.rendered_diagnostics /* upstream: $stdout */"########,
        r########"qr|WARN - Possible typo \(case mismatch\) between citation and datasource keys: 'C1' and 'c1' in file 't/tdata/full-bbl\.bib'|ms"########,
        r########"like($stdout, qr|WARN - Possible typo \(case mismatch\) between citation and datasource keys: 'C1' and 'c1' in file 't/tdata/full-bbl\.bib'|ms, 'Testing duplicate/case key warnings - 3');"########,
        UPSTREAM_SOURCE,
    );
}
