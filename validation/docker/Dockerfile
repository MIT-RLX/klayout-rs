# klayout-rs validation oracle image. Bundles OpenSTA + OpenROAD (from
# the upstream openroad/orfs base) plus the small pure-Python SPEF
# dumper used as the SPEF-parity oracle. This keeps the validation
# harness self-contained — one `docker pull` gets every reference tool
# the corpus regen needs.
FROM openroad/orfs:latest

# Bundled oracle scripts. Mounted into /work at run time would also
# work, but baking them in means contributors can `docker run` the
# image directly without the host-side venv setup.
COPY spef_dump.py /opt/oracle/spef_dump.py
RUN chmod +x /opt/oracle/spef_dump.py

# Convenience: surface the OpenROAD + OpenSTA binaries on PATH so the
# host harness can call `sta` / `openroad` instead of the long
# install path.
ENV PATH="/OpenROAD-flow-scripts/tools/install/OpenROAD/bin:${PATH}"

# Default cmd is bash so `docker run --rm IMG <cmd>` works naturally.
CMD ["bash"]
