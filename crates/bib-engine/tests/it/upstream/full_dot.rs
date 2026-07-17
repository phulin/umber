// Direct xfail translation of upstream t/full-dot.t at commit 74252e6.
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
  plan tests => 2;
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
#my $tmpfile = File::Temp->new(UNLINK => 0);
my $dot = $tmpfile->filename;
my $stdout;
my $stderr;

run3  [ $perl, 'bin/biber', '--noconf', '--nolog', '-dot-include=section,field,xdata,crossref,xref,related', '--output-format=dot', "--output-file=$dot", 't/tdata/full-dot.bcf' ], \undef, \$stdout, \$stderr;
#say $stdout;
#say $stderr;

is($? >> 8, 0, 'Full test has zero exit status');
ok(compare($dot, 't/tdata/full-dot.dot') == 0, 'Testing dot output');

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
fn assertion_002_testing_dot_output() {
    xfail_upstream(
        "Testing dot output",
        r########"in_process_session.output_bytes /* upstream: compare($dot, 't/tdata/full-dot.dot') == 0 */"########,
        r########"fixture_bytes('t/tdata/full-dot.dot')"########,
        r########"ok(compare($dot, 't/tdata/full-dot.dot') == 0, 'Testing dot output');"########,
        UPSTREAM_SOURCE,
    );
}
