# Fedora COPR's "make srpm" source method (packaging/copr/README.md):
#   make -f .copr/Makefile srpm outdir=<dir> [spec=<path>]
# Makes the source RPM from the commit COPR cloned, at the version src-tauri/tauri.conf.json
# says (written into the spec's Version), so a tag needs no edit here or in the spec.

root := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
# COPR passes spec= with whatever the package's Spec File field holds, which may be empty.
specfile := $(abspath $(if $(spec),$(spec),$(root)/packaging/copr/skykeep.spec))
outdir ?= $(root)/.copr/out
version := $(shell sed -n 's/^  "version": "\([^"]*\)",$$/\1/p' $(root)/src-tauri/tauri.conf.json)
work := $(abspath $(outdir))/.srpm-work

srpm:
	@test -n "$(version)" || { echo "no version in src-tauri/tauri.conf.json" >&2; exit 1; }
	command -v git >/dev/null && command -v rpmbuild >/dev/null || dnf -y install git rpm-build
	mkdir -p $(work)
	git -c safe.directory='*' -C $(root) archive --format=tar.gz \
	  --prefix=Skykeep-$(version)/ -o $(work)/Skykeep-$(version).tar.gz HEAD
	sed 's/^Version:.*/Version:        $(version)/' $(specfile) > $(work)/skykeep.spec
	rpmbuild -bs \
	  --define "_sourcedir $(work)" \
	  --define "_srcrpmdir $(abspath $(outdir))" \
	  $(work)/skykeep.spec
	rm -rf $(work)

.PHONY: srpm
