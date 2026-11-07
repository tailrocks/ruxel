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
die "check" unless execute_xfs('check', 1034 * 4096, 77, $id, $io) && $blocks == 100;
die "grow" unless execute_xfs('grow', 1034 * 4096, 77, $id, $io) && $blocks == 1024;
die "repeat" if execute_xfs('grow', 1034 * 4096, 77, $id, $io);
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
