#define NODEPS
#define main jpgfromraw_dcraw_hidden_main
#include "../../vendor/dcraw.c"
#undef main

int jpgfromraw_dcraw_main(int argc, const char **argv) {
  return jpgfromraw_dcraw_hidden_main(argc, argv);
}
