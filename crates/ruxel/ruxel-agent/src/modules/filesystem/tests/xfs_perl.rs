#[test]
fn synthetic_perl_ioctl_growth_and_guards() {
    let script = format!(
        "{}\n{}",
        include_str!("../xfs.pl"),
        r#"
my $blocks = 100;
my @calls;
my $id = sub { return (77, 1); };
my $io = sub {
    my ($request, $buf) = @_;
    push @calls, $request;
    if ($request == 0x80705864) {
        substr($$buf, 0, 4, pack('L', 4096));
        substr($$buf, 8, 4, pack('L', 1024));
        substr($$buf, 28, 4, pack('L', 25));
        substr($$buf, 32, 32, pack('Q4', $blocks, 0, 0, 64));
    } elsif ($request == 0x4010586e) {
        my ($target, $imax, $pad) = unpack('QLL', $$buf);
        die "bad grow payload" unless length($$buf) == 16 && $imax == 25 && $pad == 0;
        $blocks = $target;
    } else { die "unexpected ioctl"; }
};
die "preflight" if execute_xfs('preflight', 0, 77, $id, $io);
die "preflight wrote" unless join(',', @calls) eq 0x80705864;
@calls = ();
die "check" unless execute_xfs('check', 1034 * 4096, 77, $id, $io) && $blocks == 100;
die "check wrote" unless join(',', @calls) eq 0x80705864;
@calls = ();
die "grow" unless execute_xfs('grow', 1034 * 4096, 77, $id, $io) && $blocks == 1024;
die "repeat" if execute_xfs('grow', 1034 * 4096, 77, $id, $io);
die "grow sequence" unless join(',', @calls) eq join(',', 0x80705864, 0x4010586e, 0x80705864, 0x80705864);
die "64-block check" unless execute_xfs('check', 1088 * 4096, 77, $id, $io) && $blocks == 1024;
die "64-block grow" unless execute_xfs('grow', 1088 * 4096, 77, $id, $io) && $blocks == 1088;
die "64-block repeat" if execute_xfs('check', 1088 * 4096, 77, $id, $io);
$blocks = 1024;
my $old = scalar @calls;
eval { execute_xfs('grow', 1, 77, sub { return (78, 1) }, $io) };
die "identity" unless $@ =~ /identity/ && @calls == $old;
eval { execute_xfs('grow', 99 * 4096, 77, $id, $io) };
die "shrink" unless $@ =~ /shrink/;
eval { execute_xfs('check', '18446744073709551615', 77, $id, $io) };
die "overflow" unless $@ =~ /allocation group/;
my $large = "9007199254740993";
my $large_blocks = 100;
my $large_io = sub {
    my ($request, $buf) = @_;
    if ($request == 0x80705864) {
        substr($$buf, 0, 4, pack('L', 512));
        substr($$buf, 8, 4, pack('L', 4294967295));
        substr($$buf, 28, 4, pack('L', 25));
        substr($$buf, 32, 32, pack('Q4', $large_blocks, 0, 0, 64));
    } else {
        ($large_blocks) = unpack('Q', $$buf);
        die "integer precision" unless "$large_blocks" eq $large;
    }
};
die "large grow" unless execute_xfs('grow', '4611686018427388416', 77, $id, $large_io);
my $ignored = sub { my ($request, $buf) = @_; $io->($request, $buf) if $request == 0x80705864; };
eval { execute_xfs('grow', 2048 * 4096, 77, $id, $ignored) };
die "postcondition" unless $@ =~ /postcondition/;
for my $offset (40, 48, 56) {
    my @layout_calls;
    my $bad_layout = sub {
        my ($request, $buf) = @_;
        push @layout_calls, $request;
        $io->($request, $buf);
        substr($$buf, $offset, 8, pack('Q', $offset == 56 ? 0 : 1));
    };
    eval { execute_xfs('preflight', 0, 77, $id, $bad_layout) };
    die "unsupported layout" unless $@ =~ /layout unsupported/ && join(',', @layout_calls) eq 0x80705864;
}
for my $error (qw(ENOTTY EPERM EWOULDBLOCK EIO)) {
    for my $failed_request (0x80705864, 0x4010586e) {
        my @error_calls;
        my $failure = sub {
            my ($request, $buf) = @_;
            push @error_calls, $request;
            die "synthetic $error\n" if $request == $failed_request;
            $io->($request, $buf);
        };
        eval { execute_xfs('grow', 2048 * 4096, 77, $id, $failure) };
        my @expected = $failed_request == 0x80705864 ? (0x80705864) : (0x80705864, 0x4010586e);
        die "lost ioctl error" unless $@ eq "synthetic $error\n" && join(',', @error_calls) eq join(',', @expected);
    }
}
print "ok\n";
"#
    );
    let output = std::process::Command::new("perl")
        .args(["-T", "-e", &script])
        .env_remove("PERL5OPT")
        .env_remove("PERL5LIB")
        .env_remove("PERLLIB")
        .env_remove("PERL5DB")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ok\n");
}
#[test]
fn perl_and_required_core_modules_are_available() {
    let result = std::process::Command::new("perl")
        .args([
            "-T",
            "-MConfig",
            "-MFcntl",
            "-MMath::BigInt",
            "-MPOSIX",
            "-e",
            "1",
        ])
        .output()
        .expect("XFS tests require Perl and its core modules");
    assert!(
        result.status.success(),
        "XFS tests require Perl core modules: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
